use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

use nanocode_http::Url;
use nanocode_tempo::{Address, hex};
use serde::Deserialize;
use serde_json::{Value, json};

use super::accounts::{self, AccountsError, PersistedCredentials, RpcKeyAuthorization};
use super::keys::{self, KeySigner};
use super::session::channel::{OPEN_SIGNATURE, TIP20_CHANNEL_RESERVE_ADDRESS, TOP_UP_SIGNATURE};
use super::tx::{self, PATH_USD, USDC};

pub const TEMPO_ID_URL: &str = "https://id.tempo.xyz";
pub const CLIENT_ID: &str = "nanocode";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const SCOPE: &str = "openid";
const KEY_TYPE: &str = "secp256k1";
const KEY_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
const MACH_ALLOWANCE: u64 = 2_000_000;
const FEE_ALLOWANCE: u64 = 100_000;
const FUNDING_HINT: &str = "2";
const FUNDING_TOKEN: &str = "MACH";
const SLOW_DOWN: Duration = Duration::from_secs(5);
const SECRET: usize = 32;
const REQUEST_CLOSE_SIGNATURE: &str =
    "requestClose((address,address,address,address,bytes32,address,bytes32))";
const WITHDRAW_SIGNATURE: &str =
    "withdraw((address,address,address,address,bytes32,address,bytes32))";

#[derive(Debug)]
pub enum TempoIdError {
    Network(String),
    Refused {
        status: u16,
        error: String,
        description: Option<String>,
    },
    Denied,
    Expired,
    Invalid(String),
    Unsupported(u64),
    Store(io::Error),
    Accounts(AccountsError),
}

impl fmt::Display for TempoIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(error) => write!(f, "cannot reach Tempo ID: {error}"),
            Self::Refused {
                status,
                error,
                description,
            } => match description {
                Some(description) => {
                    write!(f, "Tempo ID refused ({status} {error}): {description}")
                }
                None => write!(f, "Tempo ID refused ({status} {error})"),
            },
            Self::Denied => f.write_str("the sign-in was denied in Tempo ID"),
            Self::Expired => f.write_str(
                "the Tempo ID code expired before it was approved; run /login tempo again",
            ),
            Self::Invalid(error) => write!(f, "Tempo ID answered unexpectedly: {error}"),
            Self::Unsupported(chain_id) => {
                write!(f, "nanocode has no MACH access key for chain {chain_id}")
            }
            Self::Store(error) => write!(f, "cannot save the Tempo ID sign-in: {error}"),
            Self::Accounts(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for TempoIdError {}

impl From<AccountsError> for TempoIdError {
    fn from(error: AccountsError) -> Self {
        Self::Accounts(error)
    }
}

#[derive(Deserialize)]
struct DeviceCode {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    #[serde(default)]
    interval: Option<u64>,
    expires_in: u64,
}

#[derive(Deserialize)]
struct Refusal {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct Grant {
    #[serde(default)]
    account: Option<String>,
    key_authorization: Value,
}

pub struct DeviceLogin {
    base: Url,
    device_code: String,
    user_code: String,
    verification_url: String,
    interval: Duration,
    deadline: u64,
    chain_id: u64,
    secret: [u8; SECRET],
    key_id: Address,
    path: PathBuf,
}

impl fmt::Debug for DeviceLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceLogin")
            .field("user_code", &self.user_code)
            .field("verification_url", &self.verification_url)
            .field("chain_id", &self.chain_id)
            .field("key_id", &self.key_id)
            .finish_non_exhaustive()
    }
}

pub fn access_key_request(key_id: Address, chain_id: u64, now: u64) -> Result<Value, TempoIdError> {
    let (Some(mach), Some(swapper)) = (tx::machine_token(chain_id), tx::machine_swapper(chain_id))
    else {
        return Err(TempoIdError::Unsupported(chain_id));
    };
    let selector = |signature: &str| hex::encode_prefixed(nanocode_tempo::abi::selector(signature));
    let address = |address: Address| hex::encode_prefixed(address.as_bytes());
    let quantity = |value: u64| format!("{value:#x}");
    let mut scopes = vec![
        json!({ "address": address(mach), "selector": selector(tx::SIGNATURE_TRANSFER_WITH_MEMO) }),
        json!({ "address": address(mach), "selector": selector(tx::SIGNATURE_APPROVE), "recipients": [address(swapper)] }),
        json!({ "address": address(swapper), "selector": selector(tx::SIGNATURE_SWAP_TO) }),
    ];
    scopes.extend([OPEN_SIGNATURE, TOP_UP_SIGNATURE, REQUEST_CLOSE_SIGNATURE, WITHDRAW_SIGNATURE].map(|signature| {
        json!({ "address": address(TIP20_CHANNEL_RESERVE_ADDRESS), "selector": selector(signature) })
    }));
    let fee_tokens: &[Address] = if chain_id == tx::CHAIN_ID {
        &[USDC, PATH_USD]
    } else {
        &[PATH_USD]
    };
    let mut limits = vec![json!({ "token": address(mach), "limit": quantity(MACH_ALLOWANCE) })];
    limits.extend(
        fee_tokens
            .iter()
            .map(|token| json!({ "token": address(*token), "limit": quantity(FEE_ALLOWANCE) })),
    );
    Ok(json!({
        "address": address(key_id),
        "chainId": quantity(chain_id),
        "expiry": now + KEY_LIFETIME.as_secs(),
        "keyType": KEY_TYPE,
        "limits": limits,
        "scopes": scopes,
    }))
}

fn endpoint(base: &Url, path: &str) -> Result<Url, TempoIdError> {
    base.join(path)
        .map_err(|error| TempoIdError::Invalid(format!("{path}: {error}")))
}

fn form(pairs: &[(&str, &str)]) -> Vec<u8> {
    let mut url = Url::parse("http://form.invalid/").expect("static URL parses");
    {
        let mut query = url.query_pairs_mut();
        for (name, value) in pairs {
            query.append_pair(name, value);
        }
    }
    url.query().unwrap_or_default().as_bytes().to_vec()
}

async fn post(url: Url, pairs: &[(&str, &str)]) -> Result<(u16, String), TempoIdError> {
    let response = nanocode_http::client()
        .post(url)
        .header(
            nanocode_http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(nanocode_http::header::ACCEPT, "application/json")
        .body(form(pairs))
        .send()
        .await
        .map(nanocode_http::decode_flate_response)
        .map_err(|error| TempoIdError::Network(error.to_string()))?;
    let status = response.status().as_u16();
    let text = response
        .text()
        .await
        .map_err(|error| TempoIdError::Network(error.to_string()))?;
    Ok((status, text))
}

fn refusal(status: u16, text: &str) -> TempoIdError {
    match serde_json::from_str::<Refusal>(text) {
        Ok(refusal) => TempoIdError::Refused {
            status,
            error: refusal.error,
            description: refusal.error_description,
        },
        Err(_) => TempoIdError::Refused {
            status,
            error: "unexpected_response".to_owned(),
            description: Some(text.chars().take(200).collect()),
        },
    }
}

fn device_name() -> String {
    let host = fs::read_to_string("/etc/hostname")
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .or_else(|| {
            std::env::var("HOSTNAME")
                .ok()
                .filter(|name| !name.is_empty())
        });
    match host {
        Some(host) => format!("nanocode on {host}"),
        None => "nanocode".to_owned(),
    }
}

impl DeviceLogin {
    pub async fn start(chain_id: u64) -> Result<Self, TempoIdError> {
        let base =
            Url::parse(TEMPO_ID_URL).map_err(|error| TempoIdError::Invalid(error.to_string()))?;
        Self::start_at(base, chain_id, accounts::default_credentials_path()?).await
    }

    pub async fn start_at(base: Url, chain_id: u64, path: PathBuf) -> Result<Self, TempoIdError> {
        let mut secret = [0u8; SECRET];
        nanocode_platform::fill_random(&mut secret).map_err(TempoIdError::Store)?;
        let key_id = KeySigner::secp256k1(&secret)
            .map_err(|error| TempoIdError::Invalid(error.to_string()))?
            .address();
        let request = access_key_request(key_id, chain_id, keys::unix_now())?;
        let key_authorization = nanocode_tempo::base64::encode_url(request.to_string());
        let device = device_name();
        let (status, text) = post(
            endpoint(&base, "/device/code")?,
            &[
                ("client_id", CLIENT_ID),
                ("device", &device),
                ("scope", SCOPE),
                ("key_authorization", &key_authorization),
                ("funding_hint", FUNDING_HINT),
                ("funding_token", FUNDING_TOKEN),
            ],
        )
        .await?;
        if status != 200 {
            return Err(refusal(status, &text));
        }
        let code: DeviceCode = serde_json::from_str(&text)
            .map_err(|error| TempoIdError::Invalid(error.to_string()))?;
        Ok(Self {
            base,
            verification_url: code
                .verification_uri_complete
                .unwrap_or(code.verification_uri),
            device_code: code.device_code,
            user_code: code.user_code,
            interval: Duration::from_secs(code.interval.unwrap_or(1).max(1)),
            deadline: keys::unix_now().saturating_add(code.expires_in),
            chain_id,
            secret,
            key_id,
            path,
        })
    }

    pub fn user_code(&self) -> &str {
        &self.user_code
    }

    pub fn verification_url(&self) -> &str {
        &self.verification_url
    }

    pub const fn key_id(&self) -> Address {
        self.key_id
    }

    pub async fn poll(&self) -> Result<Option<Address>, TempoIdError> {
        let (status, text) = post(
            endpoint(&self.base, "/oauth2/token")?,
            &[
                ("grant_type", DEVICE_GRANT),
                ("client_id", CLIENT_ID),
                ("device_code", &self.device_code),
            ],
        )
        .await?;
        if status == 200 {
            return self.save(&text).map(Some);
        }
        match refusal(status, &text) {
            TempoIdError::Refused { error, .. } if error == "authorization_pending" => Ok(None),
            TempoIdError::Refused { error, .. } if error == "slow_down" => {
                Err(TempoIdError::Refused {
                    status,
                    error,
                    description: None,
                })
            }
            TempoIdError::Refused { error, .. } if error == "access_denied" => {
                Err(TempoIdError::Denied)
            }
            TempoIdError::Refused { error, .. } if error == "expired_token" => {
                Err(TempoIdError::Expired)
            }
            other => Err(other),
        }
    }

    pub async fn complete(mut self) -> Result<Address, TempoIdError> {
        loop {
            if keys::unix_now() >= self.deadline {
                return Err(TempoIdError::Expired);
            }
            nanocode_platform::sleep(self.interval).await;
            match self.poll().await {
                Ok(Some(account)) => return Ok(account),
                Ok(None) => {}
                Err(TempoIdError::Refused { error, .. }) if error == "slow_down" => {
                    self.interval += SLOW_DOWN;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn save(&self, text: &str) -> Result<Address, TempoIdError> {
        let grant: Grant =
            serde_json::from_str(text).map_err(|error| TempoIdError::Invalid(error.to_string()))?;
        let authorization = RpcKeyAuthorization::deserialize(&grant.key_authorization)
            .map_err(|error| TempoIdError::Invalid(format!("key_authorization: {error}")))?;
        let signed = keys::rpc_authorization(authorization)
            .map_err(|error| TempoIdError::Invalid(format!("key_authorization: {error}")))?;
        if signed.authorization.key_id != self.key_id
            || signed.authorization.chain_id != self.chain_id
        {
            return Err(TempoIdError::Invalid(
                "key_authorization names another key or chain".to_owned(),
            ));
        }
        let account = match (signed.authorization.account, grant.account.as_deref()) {
            (Some(account), _) => account,
            (None, Some(account)) => account
                .parse()
                .map_err(|_| TempoIdError::Invalid(format!("account {account}")))?,
            (None, None) => keys::recover_signer(&signed).ok_or_else(|| {
                TempoIdError::Invalid("key_authorization names no account".to_owned())
            })?,
        };
        let credentials = json!({
            "account": hex::encode_prefixed(account.as_bytes()),
            "chain_id": self.chain_id,
            "key_type": KEY_TYPE,
            "private_key": hex::encode_prefixed(self.secret),
            "key_authorization": grant.key_authorization,
        });
        let parsed = PersistedCredentials::deserialize(&credentials)
            .map_err(|error| TempoIdError::Invalid(error.to_string()))?;
        if parsed.account.0 != account {
            return Err(TempoIdError::Invalid(
                "account does not round-trip".to_owned(),
            ));
        }
        write_private(&self.path, credentials.to_string().as_bytes())
            .map_err(TempoIdError::Store)?;
        Ok(account)
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    io::Write::write_all(&mut options.open(&temporary)?, bytes)?;
    fs::rename(&temporary, path)
}

pub fn deposit_url(account: Address, chain_id: u64, amount: Option<&str>) -> String {
    let mut url = Url::parse(TEMPO_ID_URL).expect("static URL parses");
    url.set_path("/deposit");
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("address", &hex::encode_prefixed(account.as_bytes()));
        query.append_pair(
            "chain",
            if chain_id == tx::CHAIN_ID {
                "mainnet"
            } else {
                "testnet"
            },
        );
        query.append_pair("client_id", CLIENT_ID);
        query.append_pair("funding_token", FUNDING_TOKEN);
        query.append_pair(
            "funding_hint",
            amount
                .filter(|amount| !amount.is_empty())
                .unwrap_or(FUNDING_HINT),
        );
    }
    url.to_string()
}
