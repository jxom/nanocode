use std::{
    collections::{BTreeMap, VecDeque},
    future::Future,
    num::NonZeroUsize,
    time::Duration,
};

use nanocode_sync::Atom;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HedgePolicy {
    pub cap_percent: u8,
    pub floor: Duration,
    pub quantile_percent: u8,
    pub window: NonZeroUsize,
}

impl HedgePolicy {
    pub const DEFAULT: Self = Self {
        cap_percent: 10,
        floor: Duration::from_millis(1500),
        quantile_percent: 90,
        window: NonZeroUsize::new(100).expect("the default hedge window is not zero"),
    };

    #[must_use]
    pub fn estimate_samples(self) -> usize {
        match 100usize.saturating_sub(usize::from(self.quantile_percent)) {
            0 => 1,
            tail => 100usize.div_ceil(tail),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HedgeWinner {
    Hedge,
    Primary,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[cfg_attr(not(target_family = "wasm"), derive(Deserialize))]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum HedgeOutcome {
    Capped {
        deadline_ns: u64,
    },
    Dispatched {
        deadline_ns: u64,
        hedge_first_byte_ns: Option<u64>,
        winner: HedgeWinner,
    },
    Throttled {
        deadline_ns: u64,
    },
    Unestimated,
    #[default]
    Unneeded,
}

#[cfg(target_family = "wasm")]
impl<'de> Deserialize<'de> for HedgeOutcome {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        #[derive(Deserialize)]
        struct Wire {
            status: String,
            deadline_ns: Option<u64>,
            hedge_first_byte_ns: Option<u64>,
            winner: Option<HedgeWinner>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let deadline_ns = || {
            wire.deadline_ns
                .ok_or_else(|| D::Error::missing_field("deadline_ns"))
        };
        Ok(match wire.status.as_str() {
            "capped" => Self::Capped {
                deadline_ns: deadline_ns()?,
            },
            "dispatched" => Self::Dispatched {
                deadline_ns: deadline_ns()?,
                hedge_first_byte_ns: wire.hedge_first_byte_ns,
                winner: wire
                    .winner
                    .ok_or_else(|| D::Error::missing_field("winner"))?,
            },
            "throttled" => Self::Throttled {
                deadline_ns: deadline_ns()?,
            },
            "unestimated" => Self::Unestimated,
            "unneeded" => Self::Unneeded,
            status => {
                return Err(D::Error::unknown_variant(
                    status,
                    &[
                        "capped",
                        "dispatched",
                        "throttled",
                        "unestimated",
                        "unneeded",
                    ],
                ));
            }
        })
    }
}

impl HedgeOutcome {
    #[must_use]
    pub const fn dispatched(&self) -> bool {
        matches!(self, Self::Dispatched { .. })
    }

    #[must_use]
    pub const fn unneeded(&self) -> bool {
        matches!(self, Self::Unneeded)
    }
}

pub struct Hedged<T, E> {
    pub outcome: HedgeOutcome,
    pub result: Result<T, E>,
}

#[derive(Clone, Copy)]
struct Sample {
    first_byte: Option<Duration>,
    hedged: bool,
}

#[derive(Clone, Default)]
struct Window {
    pending: usize,
    samples: VecDeque<Sample>,
}

pub struct Hedging {
    models: Atom<BTreeMap<Box<str>, Window>>,
    policy: HedgePolicy,
}

struct Reservation<'a> {
    first_byte: Option<Duration>,
    hedging: &'a Hedging,
    model: &'a str,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        self.hedging.record(self.model, self.first_byte, true);
    }
}

impl Hedging {
    #[must_use]
    pub fn new(policy: HedgePolicy) -> Self {
        Self {
            models: Atom::default(),
            policy,
        }
    }

    #[must_use]
    pub const fn policy(&self) -> HedgePolicy {
        self.policy
    }

    #[must_use]
    pub fn deadline(&self, model: &str) -> Duration {
        self.estimate(model).unwrap_or(self.policy.floor)
    }

    fn estimate(&self, model: &str) -> Option<Duration> {
        let mut times = self.models.read(|models| {
            models.get(model).map_or_else(Vec::new, |window| {
                window
                    .samples
                    .iter()
                    .filter_map(|sample| sample.first_byte)
                    .collect::<Vec<_>>()
            })
        });
        if times.len() < self.policy.estimate_samples() {
            return None;
        }
        times.sort_unstable();
        let rank = (times.len() * usize::from(self.policy.quantile_percent)).div_ceil(100);
        Some(times[rank.clamp(1, times.len()) - 1].max(self.policy.floor))
    }

    fn admit(&self, model: &str) -> bool {
        let cap = usize::from(self.policy.cap_percent);
        self.models.update(|models| {
            let window = models.entry(model.into()).or_default();
            let hedged =
                window.pending + window.samples.iter().filter(|sample| sample.hedged).count();
            let requests = window.samples.len() + window.pending + 1;
            let admitted = (hedged + 1) * 100 <= requests * cap;
            if admitted {
                window.pending += 1;
            }
            admitted
        })
    }

    fn release(&self, model: &str) {
        self.models.update(|models| {
            let window = models.entry(model.into()).or_default();
            window.pending = window.pending.saturating_sub(1);
        });
    }

    fn record(&self, model: &str, first_byte: Option<Duration>, hedged: bool) {
        let limit = self.policy.window.get();
        self.models.update(|models| {
            let window = models.entry(model.into()).or_default();
            if hedged {
                window.pending = window.pending.saturating_sub(1);
            }
            window.samples.push_back(Sample { first_byte, hedged });
            while window.samples.len() > limit {
                window.samples.pop_front();
            }
        });
    }

    pub async fn race<T, E, P, H>(
        &self,
        model: &str,
        primary: P,
        hedge: impl FnOnce() -> H,
    ) -> Hedged<T, E>
    where
        P: Future<Output = Result<T, E>>,
        H: Future<Output = Result<T, E>>,
    {
        self.race_when(model, primary, || Some(hedge())).await
    }

    pub async fn race_when<T, E, P, H>(
        &self,
        model: &str,
        primary: P,
        hedge: impl FnOnce() -> Option<H>,
    ) -> Hedged<T, E>
    where
        P: Future<Output = Result<T, E>>,
        H: Future<Output = Result<T, E>>,
    {
        let started = nanocode_platform::now();
        let Some(deadline) = self.estimate(model) else {
            let result = primary.await;
            self.record(
                model,
                result
                    .is_ok()
                    .then(|| nanocode_platform::now().saturating_duration_since(started)),
                false,
            );
            return Hedged {
                outcome: HedgeOutcome::Unestimated,
                result,
            };
        };
        let deadline_ns = u64::try_from(deadline.as_nanos()).unwrap_or(u64::MAX);
        let mut primary = std::pin::pin!(primary);
        let early = nanocode_sync::select! {
            result = primary.as_mut() => Some(result),
            () = nanocode_platform::sleep(deadline) => None,
        };
        if let Some(result) = early {
            self.record(
                model,
                result
                    .is_ok()
                    .then(|| nanocode_platform::now().saturating_duration_since(started)),
                false,
            );
            return Hedged {
                outcome: HedgeOutcome::Unneeded,
                result,
            };
        }
        if !self.admit(model) {
            let result = primary.await;
            self.record(
                model,
                result
                    .is_ok()
                    .then(|| nanocode_platform::now().saturating_duration_since(started)),
                false,
            );
            return Hedged {
                outcome: HedgeOutcome::Capped { deadline_ns },
                result,
            };
        }
        let Some(hedge) = hedge() else {
            self.release(model);
            let result = primary.await;
            self.record(
                model,
                result
                    .is_ok()
                    .then(|| nanocode_platform::now().saturating_duration_since(started)),
                false,
            );
            return Hedged {
                outcome: HedgeOutcome::Throttled { deadline_ns },
                result,
            };
        };
        let mut reservation = Reservation {
            first_byte: None,
            hedging: self,
            model,
        };
        let hedge_started = nanocode_platform::now();
        let mut hedge = std::pin::pin!(hedge);
        let first = nanocode_sync::select! {
            result = primary.as_mut() => (HedgeWinner::Primary, result),
            result = hedge.as_mut() => (HedgeWinner::Hedge, result),
        };
        let (winner, result) = match first {
            (HedgeWinner::Primary, Err(error)) => match hedge.await {
                Ok(value) => (HedgeWinner::Hedge, Ok(value)),
                Err(_) => (HedgeWinner::Primary, Err(error)),
            },
            (HedgeWinner::Hedge, Err(error)) => match primary.await {
                Ok(value) => (HedgeWinner::Primary, Ok(value)),
                Err(_) => (HedgeWinner::Hedge, Err(error)),
            },
            settled => settled,
        };
        reservation.first_byte = result.is_ok().then(|| match winner {
            HedgeWinner::Hedge => nanocode_platform::now().saturating_duration_since(hedge_started),
            HedgeWinner::Primary => nanocode_platform::now().saturating_duration_since(started),
        });
        let hedge_first_byte_ns = reservation
            .first_byte
            .filter(|_| winner == HedgeWinner::Hedge)
            .map(|elapsed| u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX));
        drop(reservation);
        Hedged {
            outcome: HedgeOutcome::Dispatched {
                deadline_ns,
                hedge_first_byte_ns,
                winner,
            },
            result,
        }
    }
}
