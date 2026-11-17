use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, VecDeque},
    ops::Range,
    sync::Arc,
};

use nanocode_durability_core::{SqlStatement, StoreError};
use nanocode_sync::{Held, Owned, mpsc, oneshot};

use crate::{
    Checkpoint, Compaction, Event, Extension, Fold, LogHost, Record, Rider, Segment, Write,
    codec::{
        self, ARCHIVE, BOUNDARY, History, PART, RECORDS, SEAL, SECTION, SNAPSHOT, Stream, WINDOW,
    },
    fold::{Entry, Page, Section, legacy_name},
};

pub const EVENT_GAP: u64 = 1 << 14;
const CLAIM: &str = "writer";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Point {
    Admission,
    Call,
    Cancel,
    Finish,
    Floor,
    Progress,
    Recovery,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Stats {
    pub admissions: u64,
    pub boundaries: u64,
    pub calls: u64,
    pub cancels: u64,
    pub collected: u64,
    pub finishes: u64,
    pub floors: u64,
    pub index_loads: u64,
    pub page_rows: u64,
    pub pages: u64,
    pub parts: u64,
    pub plain_bytes: u64,
    pub progress: u64,
    pub recoveries: u64,
    pub retired: u64,
    pub row_bytes: u64,
    pub section_loads: u64,
    pub sections: u64,
    pub snapshot_bytes: u64,
    pub snapshots: u64,
}

impl Stats {
    #[must_use]
    pub const fn rows(&self) -> u64 {
        self.admissions
            + self.calls
            + self.cancels
            + self.finishes
            + self.floors
            + self.parts
            + self.progress
            + self.recoveries
            + self.sections
    }
}

#[derive(Debug)]
pub struct Row {
    pub boundary: bool,
    pub bytes: usize,
    pub plain: String,
    pub seq: u64,
    pub snapshot: bool,
}

impl Row {
    pub fn records(&self) -> Result<Vec<Record>, StoreError> {
        crate::decode(&self.plain)
    }
}

struct Snapshot {
    base: usize,
    packed: String,
    plain: usize,
    start: usize,
    tail: Vec<u8>,
}

impl Snapshot {
    fn suffix(&self) -> &[u8] {
        &self.tail[self.start - self.base..]
    }

    const fn trimmed(&self) -> bool {
        self.start > 0
    }
}

struct Partitioned {
    head: Snapshot,
    index: Index,
    rewrite: usize,
    sections: Sections,
    seq: u64,
}

struct Sections {
    bytes: usize,
    entries: BTreeMap<String, Option<Entry>>,
    groups: Vec<(Section, u64)>,
    next: u64,
    plain: u64,
    rewritten: u64,
    rows: Vec<(u64, String)>,
}

struct Index {
    bytes: usize,
    groups: BTreeMap<u64, (u64, u64)>,
    next: u64,
    pages: BTreeMap<String, Page>,
    root: Option<Section>,
    rows: Vec<(u64, String)>,
    superseded: Vec<Range<u64>>,
}

impl Index {
    fn seal(
        &mut self,
        spare: &mut codec::Spare,
        first: String,
        packer: codec::Packer,
        limit: Option<usize>,
    ) -> Result<(), StoreError> {
        let rows = stored(self.next, packer.finish(SECTION, spare)?.packed, limit)?;
        self.bytes += rows.iter().map(|(_, body)| body.len()).sum::<usize>();
        self.pages.insert(
            first,
            Page::Loaded(Section {
                row: self.next,
                rows: rows.len() as u64,
            }),
        );
        self.next += rows.len() as u64;
        self.rows.extend(rows);
        Ok(())
    }
}

impl Sections {
    fn seal(
        &mut self,
        spare: &mut codec::Spare,
        packer: codec::Packer,
        owners: Vec<(String, Entry)>,
        limit: Option<usize>,
    ) -> Result<(), StoreError> {
        let rows = stored(self.next, packer.finish(SECTION, spare)?.packed, limit)?;
        self.bytes += rows.iter().map(|(_, body)| body.len()).sum::<usize>();
        let section = Section {
            row: self.next,
            rows: rows.len() as u64,
        };
        self.groups.push((section, owners.len() as u64));
        for (owner, mut entry) in owners {
            entry.sections.push(section);
            self.entries.insert(owner, Some(entry));
        }
        self.next += rows.len() as u64;
        self.rows.extend(rows);
        Ok(())
    }
}

fn line(lines: &mut Vec<u8>, record: &Record) -> Result<(), StoreError> {
    if !lines.is_empty() {
        lines.push(b'\n');
    }
    crate::record::write(record, lines)
}

fn sections(
    spare: &mut codec::Spare,
    fold: &Fold,
    from: u64,
    level: i32,
    limit: Option<usize>,
    ratio: usize,
) -> Result<Sections, StoreError> {
    let mut sections = Sections {
        bytes: 0,
        entries: BTreeMap::new(),
        groups: Vec::new(),
        next: from,
        plain: 0,
        rewritten: 0,
        rows: Vec::new(),
    };
    let mut open: Option<(codec::Packer, Vec<(String, Entry)>)> = None;
    let mut lines = Vec::new();
    for owner in fold.writes() {
        lines.clear();
        let changed = fold.delta_records(owner, |record| line(&mut lines, &record))?;
        let base = fold.entry(owner).cloned().unwrap_or_default();
        let full = fold.loaded(owner)
            && (base.sections.is_empty()
                || base.written + lines.len() as u64 >= ratio as u64 * base.full);
        let checkpoint = if full {
            lines.clear();
            fold.owner_records(owner, |record| line(&mut lines, &record))?
        } else {
            base.checkpoint || changed
        };
        let bytes = lines.len() as u64;
        sections.plain += bytes;
        if full && !base.sections.is_empty() {
            sections.rewritten += bytes;
        }
        if lines.is_empty() {
            sections
                .entries
                .insert(owner.to_owned(), (!full).then_some(base));
            continue;
        }
        let entry = if full {
            Entry {
                checkpoint,
                full: bytes,
                legacy: false,
                sections: Vec::new(),
                written: bytes,
            }
        } else {
            Entry {
                checkpoint,
                full: base.full,
                legacy: base.legacy,
                sections: base.sections,
                written: base.written + bytes,
            }
        };
        let (packer, owners) = match &mut open {
            Some(open) => open,
            None => open.insert((codec::Packer::new(spare, level, &[])?, Vec::new())),
        };
        if packer.plain() > 0 {
            packer.write(b"\n")?;
        }
        packer.write(&lines)?;
        owners.push((owner.to_owned(), entry));
        if packer.plain() >= WINDOW
            && let Some((packer, owners)) = open.take()
        {
            sections.seal(spare, packer, owners, limit)?;
        }
    }
    if let Some((packer, owners)) = open.take() {
        sections.seal(spare, packer, owners, limit)?;
    }
    Ok(sections)
}

fn index(
    spare: &mut codec::Spare,
    fold: &Fold,
    sections: &Sections,
    level: i32,
    limit: Option<usize>,
) -> Result<Index, StoreError> {
    let plan = fold.plan(&sections.entries, &sections.groups)?;
    let mut index = Index {
        bytes: 0,
        groups: plan.groups,
        next: sections.next,
        pages: plan.kept,
        root: None,
        rows: Vec::new(),
        superseded: plan.superseded,
    };
    let mut lines = Vec::new();
    for (first, records) in plan.pages {
        let mut fence = Some(first);
        let mut open: Option<(String, codec::Packer)> = None;
        for record in records {
            let Record::Owner { owner, .. } = &record else {
                continue;
            };
            let (_, packer) = match &mut open {
                Some(open) => open,
                None => open.insert((
                    fence.take().unwrap_or_else(|| owner.clone()),
                    codec::Packer::new(spare, level, &[])?,
                )),
            };
            lines.clear();
            line(&mut lines, &record)?;
            if packer.plain() > 0 {
                packer.write(b"\n")?;
            }
            packer.write(&lines)?;
            if packer.plain() >= WINDOW
                && let Some((first, packer)) = open.take()
            {
                index.seal(spare, first, packer, limit)?;
            }
        }
        if let Some((first, packer)) = open.take() {
            index.seal(spare, first, packer, limit)?;
        }
    }
    if let Some((first, _)) = index.pages.first_key_value()
        && !first.is_empty()
        && let Some((_, page)) = index.pages.pop_first()
    {
        index.pages.insert(String::new(), page);
    }
    index.root = match fold.indexed(&index.pages, &index.groups) {
        Some(section) => Some(section),
        None => {
            if let Some(old) = fold.root_section() {
                index.superseded.push(old.row..old.row + old.rows);
                index.superseded.sort_by_key(|range| range.start);
            }
            if index.pages.is_empty() && index.groups.is_empty() {
                None
            } else {
                let mut packer = codec::Packer::new(spare, level, &[])?;
                for (first, page) in &index.pages {
                    let section = page.section();
                    lines.clear();
                    line(
                        &mut lines,
                        &Record::Index {
                            first: first.clone(),
                            row: section.row,
                            rows: section.rows,
                        },
                    )?;
                    if packer.plain() > 0 {
                        packer.write(b"\n")?;
                    }
                    packer.write(&lines)?;
                }
                if !index.groups.is_empty() {
                    lines.clear();
                    line(
                        &mut lines,
                        &Record::Groups {
                            groups: index
                                .groups
                                .iter()
                                .map(|(&row, &(rows, count))| (row, rows, count))
                                .collect(),
                        },
                    )?;
                    if packer.plain() > 0 {
                        packer.write(b"\n")?;
                    }
                    packer.write(&lines)?;
                }
                let rows = stored(index.next, packer.finish(SECTION, spare)?.packed, limit)?;
                index.bytes += rows.iter().map(|(_, body)| body.len()).sum::<usize>();
                let section = Section {
                    row: index.next,
                    rows: rows.len() as u64,
                };
                index.next += rows.len() as u64;
                index.rows.extend(rows);
                Some(section)
            }
        }
    };
    Ok(index)
}

async fn section_records(
    host: &dyn LogHost,
    scope: &str,
    owner: &str,
    section: Section,
) -> Result<Vec<Record>, StoreError> {
    let end = section.row + section.rows;
    let stored = host.scan(scope, section.row, end).await?;
    if stored.first().map(|(seq, _)| *seq) != Some(section.row)
        || stored.len() as u64 != section.rows
        || !stored[0].1.starts_with(SECTION)
    {
        return Err(StoreError::Backend(format!(
            "turn log section of {owner} at rows {}..{end} reads {} rows",
            section.row,
            stored.len()
        )));
    }
    let mut records = Vec::new();
    each_row(stored, &mut History::default(), |row| {
        crate::record::each(&row.plain, |record| {
            records.push(record);
            Ok(())
        })?;
        Ok(true)
    })?;
    Ok(records)
}

async fn root(
    host: &dyn LogHost,
    scope: &str,
    fold: &mut Fold,
    stats: &mut Stats,
) -> Result<(), StoreError> {
    if let Some(section) = fold.cold_root() {
        let records = section_records(host, scope, "the index root", section).await?;
        fold.rooted(records)?;
        stats.index_loads += 1;
    }
    Ok(())
}

async fn page(
    host: &dyn LogHost,
    scope: &str,
    fold: &mut Fold,
    stats: &mut Stats,
    owner: &str,
) -> Result<(), StoreError> {
    root(host, scope, fold, stats).await?;
    if let Some((first, section)) = fold.unpaged(owner) {
        let records = section_records(host, scope, &first, section).await?;
        fold.paged(&first, records)?;
        stats.index_loads += 1;
    }
    Ok(())
}

async fn warm(
    host: &dyn LogHost,
    scope: &str,
    fold: &mut Fold,
    stats: &mut Stats,
    owner: &str,
) -> Result<(), StoreError> {
    if let Some(legacy) = legacy_name(owner) {
        page(host, scope, fold, stats, &legacy).await?;
        if fold.is_legacy(&legacy) {
            load(host, scope, fold, stats, &legacy).await?;
        }
    }
    load(host, scope, fold, stats, owner).await
}

async fn load(
    host: &dyn LogHost,
    scope: &str,
    fold: &mut Fold,
    stats: &mut Stats,
    owner: &str,
) -> Result<(), StoreError> {
    page(host, scope, fold, stats, owner).await?;
    if fold.is_legacy(owner) {
        for (first, section) in fold.key_pages(owner.trim_end_matches('\n')) {
            let records = section_records(host, scope, &first, section).await?;
            fold.paged(&first, records)?;
            stats.index_loads += 1;
        }
    }
    let Some(sections) = fold.cold(owner).map(<[Section]>::to_vec) else {
        return Ok(());
    };
    let legacy = fold.is_legacy(owner);
    let mut groups = Vec::with_capacity(sections.len());
    for section in sections {
        let records = section_records(host, scope, owner, section).await?;
        if !legacy {
            for (first, page) in fold.group_pages(&records) {
                let listed = section_records(host, scope, &first, page).await?;
                fold.paged(&first, listed)?;
                stats.index_loads += 1;
            }
        }
        groups.push((section, records));
    }
    stats.section_loads += groups.len() as u64;
    fold.warm(owner, groups)
}

async fn warm_state(
    host: &dyn LogHost,
    scope: &str,
    fold: &mut Fold,
    stats: &mut Stats,
    state: &str,
) -> Result<(), StoreError> {
    warm(host, scope, fold, stats, state).await?;
    for (first, section) in fold.key_pages(state) {
        let records = section_records(host, scope, &first, section).await?;
        fold.paged(&first, records)?;
        stats.index_loads += 1;
    }
    for key in fold.cold_keys(state) {
        warm(host, scope, fold, stats, &key).await?;
    }
    fold.touch(state);
    Ok(())
}

fn packed(
    spare: &mut codec::Spare,
    fold: &Fold,
    keep: u64,
    level: i32,
    index: Option<Section>,
) -> Result<Snapshot, StoreError> {
    let mut packer = codec::Packer::new(spare, level, &[])?;
    let mut line = Vec::new();
    let mut base = 0;
    let mut starts = VecDeque::new();
    let mut tail = Vec::new();
    fold.head_records(keep, index, |record| {
        if packer.plain() > 0 {
            line.push(b'\n');
        }
        let start = base + tail.len() + line.len();
        crate::record::write(&record, &mut line)?;
        packer.write(&line)?;
        tail.extend_from_slice(&line);
        line.clear();
        starts.push_back((start, matches!(record, Record::Event { .. })));
        let end = base + tail.len();
        while let (Some(&(_, false)), Some(&(next, _))) = (starts.front(), starts.get(1)) {
            if end - next < WINDOW {
                break;
            }
            starts.pop_front();
        }
        if let Some(&(front, _)) = starts.front()
            && front - base > end - front
        {
            tail.drain(..front - base);
            base = front;
        }
        Ok(())
    })?;
    let start = starts.front().map_or(0, |&(start, _)| start);
    let packed = packer.finish(SNAPSHOT, spare)?;
    Ok(Snapshot {
        base,
        packed: packed.packed,
        plain: packed.plain,
        start,
        tail,
    })
}

fn measured(fold: &Fold, keep: u64) -> Result<usize, StoreError> {
    let mut bytes = 0;
    let mut line = Vec::new();
    fold.head_records(keep, None, |record| {
        crate::record::write(&record, &mut line)?;
        bytes += line.len() + usize::from(bytes > 0);
        line.clear();
        Ok(())
    })?;
    Ok(bytes)
}

pub fn rows(stored: &[(u64, String)]) -> Result<Vec<Row>, StoreError> {
    let mut decoded = Vec::new();
    each_row(stored.to_vec(), &mut History::default(), |row| {
        decoded.push(row);
        Ok(true)
    })?;
    Ok(decoded)
}

struct Logical {
    bytes: usize,
    seq: u64,
    tag: char,
    text: String,
}

fn each_row(
    stored: Vec<(u64, String)>,
    history: &mut History,
    mut take: impl FnMut(Row) -> Result<bool, StoreError>,
) -> Result<bool, StoreError> {
    let mut pending: Option<Logical> = None;
    for (seq, body) in stored {
        let mut chars = body.chars();
        match chars.next() {
            Some(PART) => match &mut pending {
                Some(logical) => {
                    logical.text.push_str(chars.as_str());
                    logical.bytes += body.len();
                }
                None => {
                    return Err(StoreError::Backend(format!(
                        "turn log row {seq} continues no row"
                    )));
                }
            },
            Some(tag @ (ARCHIVE | BOUNDARY | RECORDS | SECTION | SNAPSHOT)) => {
                if let Some(logical) = pending.take()
                    && !take(decode_row(logical, history)?)?
                {
                    return Ok(false);
                }
                pending = Some(Logical {
                    bytes: body.len(),
                    seq,
                    tag,
                    text: body,
                });
            }
            _ => {
                return Err(StoreError::Backend(format!(
                    "turn log row {seq} has no known tag"
                )));
            }
        }
    }
    match pending {
        Some(logical) => take(decode_row(logical, history)?),
        None => Ok(true),
    }
}

fn decode_row(logical: Logical, history: &mut History) -> Result<Row, StoreError> {
    let Logical {
        bytes,
        seq,
        tag,
        text,
    } = logical;
    let snapshot = tag == SNAPSHOT;
    let boundary = snapshot || tag == BOUNDARY || tag == ARCHIVE || tag == SECTION;
    if boundary {
        history.clear();
    }
    let plain = codec::unpack(&text[tag.len_utf8()..], history.bytes())?;
    drop(text);
    history.extend(&plain);
    let plain = String::from_utf8(plain).map_err(|error| {
        StoreError::Backend(format!("turn log row {seq} is not UTF-8: {error}"))
    })?;
    Ok(Row {
        boundary,
        bytes,
        plain,
        seq,
        snapshot,
    })
}

fn stored(
    seq: u64,
    mut body: String,
    limit: Option<usize>,
) -> Result<Vec<(u64, String)>, StoreError> {
    let parts = codec::parts(&body, limit)?;
    let first = parts[0].len();
    let rest = parts[1..]
        .iter()
        .zip(seq + 1..)
        .map(|(part, row)| (row, format!("{PART}{part}")))
        .collect::<Vec<_>>();
    if !rest.is_empty() {
        body.truncate(first);
        body.shrink_to_fit();
    }
    Ok(std::iter::once((seq, body)).chain(rest).collect())
}

async fn scan(
    host: &dyn LogHost,
    scope: &str,
    stats: &mut Stats,
    range: (Segment, u64),
    after: u64,
    take: &mut impl FnMut(&Event) -> bool,
) -> Result<bool, StoreError> {
    let (segment, end) = range;
    if segment.last <= after {
        return Ok(true);
    }
    let stored = host.scan(scope, segment.row, end).await?;
    stats.pages += 1;
    stats.page_rows += stored.len() as u64;
    each_row(stored, &mut History::default(), |row| {
        for record in row.records()? {
            if let Record::Event {
                at,
                event,
                seq,
                turn,
            } = record
                && seq > after
                && seq <= segment.last
                && !take(&Event {
                    at,
                    event,
                    seq,
                    turn,
                })
            {
                return Ok(false);
            }
        }
        Ok(true)
    })
}

fn suffix(plain: &str, events: usize) -> Option<&str> {
    let floor = plain.len().saturating_sub(WINDOW);
    if floor == 0 {
        return None;
    }
    let (mut start, mut cut, mut found) = (plain.len(), plain.len(), 0);
    while start > floor || found < events {
        let boundary = plain[..cut].rfind('\n')? + 1;
        if let Ok(records) = crate::decode(&plain[boundary..start]) {
            found += records
                .iter()
                .filter(|record| matches!(record, Record::Event { .. }))
                .count();
            start = boundary;
        }
        cut = boundary - 1;
    }
    Some(&plain[start..])
}

struct Previous {
    bytes: usize,
    parts: usize,
    seq: u64,
    suffix: String,
}

impl Previous {
    fn of(seq: u64, parts: usize, bytes: usize, plain: &str, events: usize) -> Option<Self> {
        suffix(plain, events).map(|suffix| Self {
            bytes,
            parts,
            seq,
            suffix: suffix.to_owned(),
        })
    }

    fn archive(
        &self,
        spare: &mut codec::Spare,
        limit: Option<usize>,
    ) -> Result<Option<Vec<(u64, String)>>, StoreError> {
        let body = codec::pack(spare, ARCHIVE, self.suffix.as_bytes(), &[])?;
        let rows = stored(self.seq, body, limit)?;
        let bytes = rows.iter().map(|(_, body)| body.len()).sum::<usize>();
        Ok((bytes < self.bytes && rows.len() <= self.parts).then_some(rows))
    }
}

struct TailRow {
    bytes: usize,
    seq: u64,
}

pub struct Follower {
    decoded: History,
    first: Option<u64>,
    fold: Fold,
    held: Vec<(u64, String)>,
    history: History,
    host: Arc<dyn LogHost>,
    next: u64,
    previous: Option<Previous>,
    scope: Arc<str>,
    snapshot: usize,
    snapshot_row: Option<u64>,
    state: usize,
    stats: Stats,
    tail: usize,
}

impl Follower {
    pub async fn advance(&mut self) -> Result<usize, StoreError> {
        let from = self.held.first().map_or(self.next, |(seq, _)| *seq);
        let stored = self
            .host
            .scan(&self.scope, from, i64::MAX.unsigned_abs())
            .await?;
        let taken = self.take(stored)?;
        for owner in self.fold.take_forgets() {
            warm(
                &*self.host,
                &self.scope,
                &mut self.fold,
                &mut self.stats,
                &owner,
            )
            .await?;
        }
        Ok(taken)
    }

    pub async fn claim(mut self) -> Result<TurnLog, StoreError> {
        self.advance().await?;
        let log = self.finish().await?;
        let mut lease = log.lease().await;
        let next = lease.writer.next;
        lease.push(Record::Host {
            key: CLAIM.to_owned(),
            value: serde_json::value::to_raw_value(&next)
                .map_err(|error| StoreError::Backend(format!("turn log claim: {error}")))?,
        })?;
        let check = std::mem::replace(&mut lease.writer.check, usize::MAX);
        let claimed = lease.commit(Point::Recovery).await;
        lease.writer.check = check;
        claimed?;
        drop(lease);
        Ok(log)
    }

    #[must_use]
    pub const fn next(&self) -> u64 {
        self.next
    }

    fn take(&mut self, stored: Vec<(u64, String)>) -> Result<usize, StoreError> {
        let (Some(&(first, _)), Some(&(last, _))) = (stored.first(), stored.last()) else {
            return Ok(0);
        };
        if self.first.is_none() {
            self.first = Some(first);
            self.fold.start_segment(first);
        }
        self.next = last + 1;
        let mut section = false;
        let mut stored: Vec<(u64, String)> = stored
            .into_iter()
            .filter(|(_, body)| {
                if !body.starts_with(PART) {
                    section = body.starts_with(SECTION);
                }
                !section
            })
            .collect();
        if stored.is_empty() {
            self.held = Vec::new();
            return Ok(0);
        }
        let start = stored
            .iter()
            .rposition(|(_, body)| !body.starts_with(PART))
            .ok_or_else(|| {
                StoreError::Backend(format!("turn log row {} continues no row", stored[0].0))
            })?;
        self.held = stored.split_off(start);
        let taken = stored.len();
        self.fold_rows(stored, None)?;
        Ok(taken)
    }

    fn fold_rows(
        &mut self,
        stored: Vec<(u64, String)>,
        extensible: Option<TailRow>,
    ) -> Result<Option<TailRow>, StoreError> {
        let mut extensible = extensible;
        let mut tail_row = None;
        let last = stored
            .iter()
            .rposition(|(_, body)| body.starts_with(SNAPSHOT))
            .map(|index| {
                let parts = stored[index + 1..]
                    .iter()
                    .take_while(|(_, body)| body.starts_with(PART))
                    .count();
                (stored[index].0, parts + 1)
            });
        each_row(stored, &mut self.decoded, |row| {
            if row.snapshot {
                self.fold = Fold::default();
                self.fold.start_segment(row.seq);
                self.history.clear();
                self.snapshot = row.bytes;
                self.snapshot_row = Some(row.seq);
                self.state = row.plain.len();
                self.tail = 0;
            } else {
                if row.boundary {
                    let keep = row
                        .records()?
                        .iter()
                        .find_map(|record| match record {
                            Record::Event { seq, .. } => Some(*seq),
                            _ => None,
                        })
                        .unwrap_or(u64::MAX);
                    self.fold.seal(row.seq, keep);
                    self.history.clear();
                }
                self.tail += row.bytes;
            }
            if extensible
                .as_ref()
                .is_some_and(|candidate| candidate.seq == row.seq)
                && !row
                    .records()?
                    .iter()
                    .any(|record| matches!(record, Record::Recovery { .. }))
            {
                tail_row = extensible.take();
            }
            self.fold.load(row.seq, &row.plain)?;
            if let Some((seq, parts)) = last
                && row.seq == seq
            {
                self.previous =
                    Previous::of(seq, parts, row.bytes, &row.plain, self.fold.events().len());
            }
            self.fold.release();
            self.history.extend(row.plain.as_bytes());
            Ok(true)
        })?;
        Ok(tail_row)
    }

    async fn finish(mut self) -> Result<TurnLog, StoreError> {
        let held = std::mem::take(&mut self.held);
        let extensible = held
            .last()
            .filter(|(_, body)| codec::extensible(body))
            .map(|(seq, body)| TailRow {
                bytes: body.len(),
                seq: *seq,
            });
        let tail_row = self.fold_rows(held, extensible)?;
        let next = self.next;
        let tail_rows = usize::try_from(match (self.snapshot_row, self.first) {
            (Some(row), _) => next - 1 - row,
            (None, first) => first.map_or(0, |first| next - first),
        })
        .map_err(|error| StoreError::Backend(format!("turn log tail row count: {error}")))?;
        let Self {
            mut fold,
            history,
            host,
            previous,
            scope,
            snapshot,
            state,
            mut stats,
            tail,
            ..
        } = self;
        for owner in fold.forgetting() {
            warm(&*host, &scope, &mut fold, &mut stats, &owner).await?;
        }
        fold.reopen(EVENT_GAP);
        let floor = Some(fold.last_event());
        let (riding, riders) = mpsc::unbounded_channel();
        Ok(TurnLog {
            host,
            limit: None,
            ratio: 2,
            retain: None,
            riding,
            rows: None,
            scope,
            whole: false,
            writer: Owned::new(Writer {
                bases: BTreeMap::new(),
                buffer: Vec::new(),
                check: snapshot,
                collectable: BTreeSet::new(),
                floor,
                fold,
                history,
                next,
                previous,
                riders,
                snapshot,
                spare: codec::Spare::default(),
                state,
                stale: false,
                stats,
                stream: None,
                tail,
                tail_row,
                tail_rows,
                tails: BTreeMap::new(),
                tried: None,
            }),
        })
    }
}

struct Riding {
    host: usize,
    statements: Vec<(&'static str, Vec<String>)>,
    rode: oneshot::Sender<bool>,
}

struct Writer {
    bases: BTreeMap<String, Option<Checkpoint>>,
    buffer: Vec<Record>,
    check: usize,
    collectable: BTreeSet<String>,
    floor: Option<u64>,
    fold: Fold,
    history: History,
    next: u64,
    previous: Option<Previous>,
    riders: mpsc::UnboundedReceiver<Riding>,
    snapshot: usize,
    spare: codec::Spare,
    state: usize,
    stale: bool,
    stats: Stats,
    stream: Option<Stream>,
    tail: usize,
    tail_row: Option<TailRow>,
    tail_rows: usize,
    tails: BTreeMap<String, u64>,
    tried: Option<(usize, usize)>,
}

pub struct TurnLog {
    host: Arc<dyn LogHost>,
    limit: Option<usize>,
    ratio: usize,
    retain: Option<usize>,
    riding: mpsc::UnboundedSender<Riding>,
    rows: Option<usize>,
    scope: Arc<str>,
    whole: bool,
    writer: Owned<Writer>,
}

pub struct Lease<'a> {
    host: &'a dyn LogHost,
    limit: Option<usize>,
    ratio: usize,
    retain: Option<usize>,
    rows: Option<usize>,
    scope: &'a str,
    whole: bool,
    writer: Held<'a, Writer>,
}

impl TurnLog {
    pub async fn open(
        host: Arc<dyn LogHost>,
        scope: impl Into<Arc<str>>,
    ) -> Result<Self, StoreError> {
        Self::follow(host, scope).await?.finish().await
    }

    pub async fn follow(
        host: Arc<dyn LogHost>,
        scope: impl Into<Arc<str>>,
    ) -> Result<Follower, StoreError> {
        let scope = scope.into();
        let stored = host.load(&scope).await?;
        let mut follower = Follower {
            decoded: History::default(),
            first: None,
            fold: Fold::default(),
            held: Vec::new(),
            history: History::default(),
            host,
            next: 1,
            previous: None,
            scope,
            snapshot: 0,
            snapshot_row: None,
            state: 0,
            stats: Stats::default(),
            tail: 0,
        };
        follower.take(stored)?;
        Ok(follower)
    }

    #[must_use]
    pub const fn snapshot_ratio(mut self, ratio: usize) -> Self {
        self.ratio = ratio;
        self
    }

    #[must_use]
    pub const fn retain_finished(mut self, keep: Option<usize>) -> Self {
        self.retain = keep;
        self
    }

    #[must_use]
    pub const fn snapshot_rows(mut self, rows: Option<usize>) -> Self {
        self.rows = rows;
        self
    }

    #[must_use]
    pub const fn row_limit(mut self, limit: Option<usize>) -> Self {
        self.limit = limit;
        self
    }

    #[must_use]
    pub const fn whole_checkpoints(mut self, whole: bool) -> Self {
        self.whole = whole;
        self
    }

    #[must_use]
    pub fn scope(&self) -> &str {
        &self.scope
    }

    pub fn ride(
        &self,
        host: usize,
        statements: Vec<(&'static str, Vec<String>)>,
    ) -> oneshot::Receiver<bool> {
        let (rode, receiver) = oneshot::channel();
        if let Err(error) = self.riding.send(Riding {
            host,
            statements,
            rode,
        }) {
            let _ = error.0.rode.send(false);
        }
        receiver
    }

    pub async fn lease(&self) -> Lease<'_> {
        Lease {
            host: &*self.host,
            limit: self.limit,
            ratio: self.ratio,
            retain: self.retain,
            rows: self.rows,
            scope: &self.scope,
            whole: self.whole,
            writer: self.writer.acquire().await,
        }
    }

    pub async fn read<R>(&self, read: impl FnOnce(&Fold) -> R) -> R {
        read(&self.writer.acquire().await.fold)
    }

    pub async fn load_owner(&self, owner: &str) -> Result<(), StoreError> {
        let mut writer = self.writer.acquire().await;
        let writer = &mut *writer;
        warm_state(
            &*self.host,
            &self.scope,
            &mut writer.fold,
            &mut writer.stats,
            owner,
        )
        .await
    }

    pub async fn load_index(&self) -> Result<(), StoreError> {
        let mut writer = self.writer.acquire().await;
        let writer = &mut *writer;
        root(
            &*self.host,
            &self.scope,
            &mut writer.fold,
            &mut writer.stats,
        )
        .await?;
        for (first, section) in writer.fold.cold_pages() {
            let records = section_records(&*self.host, &self.scope, &first, section).await?;
            writer.fold.paged(&first, records)?;
            writer.stats.index_loads += 1;
        }
        Ok(())
    }

    pub async fn read_owners<R>(
        &self,
        owners: &[String],
        read: impl FnOnce(&Fold) -> R,
    ) -> Result<R, StoreError> {
        let mut writer = self.writer.acquire().await;
        let writer = &mut *writer;
        for owner in owners {
            warm(
                &*self.host,
                &self.scope,
                &mut writer.fold,
                &mut writer.stats,
                owner,
            )
            .await?;
        }
        Ok(read(&writer.fold))
    }

    pub async fn read_owner<R>(
        &self,
        owner: &str,
        read: impl FnOnce(&Fold) -> R,
    ) -> Result<R, StoreError> {
        let mut writer = self.writer.acquire().await;
        let writer = &mut *writer;
        warm_state(
            &*self.host,
            &self.scope,
            &mut writer.fold,
            &mut writer.stats,
            owner,
        )
        .await?;
        Ok(read(&writer.fold))
    }

    pub async fn stats(&self) -> Stats {
        self.writer.acquire().await.stats
    }

    pub async fn history(
        &self,
        after: u64,
        mut take: impl FnMut(&Event) -> bool,
    ) -> Result<(), StoreError> {
        let mut writer = self.writer.acquire().await;
        let unsealed = writer.fold.unsealed();
        let end = match unsealed {
            Some(_) => writer.next,
            None => writer.fold.segment_row(),
        };
        let mut segments = writer.fold.segments().to_vec();
        segments.extend(unsealed);
        let ends = segments
            .iter()
            .skip(1)
            .map(|segment| segment.row)
            .chain([end])
            .collect::<Vec<_>>();
        for (segment, end) in segments.into_iter().zip(ends) {
            if !scan(
                &*self.host,
                &self.scope,
                &mut writer.stats,
                (segment, end),
                after,
                &mut take,
            )
            .await?
            {
                return Ok(());
            }
        }
        for event in writer.fold.events() {
            if event.seq > after && !take(event) {
                return Ok(());
            }
        }
        Ok(())
    }
}

impl Lease<'_> {
    pub async fn held(
        &mut self,
        after: u64,
        mut take: impl FnMut(&Event) -> bool,
    ) -> Result<(), StoreError> {
        let writer = &mut *self.writer;
        if let Some(segment) = writer.fold.unsealed()
            && !scan(
                self.host,
                self.scope,
                &mut writer.stats,
                (segment, writer.next),
                after,
                &mut take,
            )
            .await?
        {
            return Ok(());
        }
        for event in writer.fold.events() {
            if event.seq > after && !take(event) {
                return Ok(());
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn fold(&self) -> &Fold {
        &self.writer.fold
    }

    pub async fn load_owners(&mut self, owners: &[String]) -> Result<(), StoreError> {
        let writer = &mut *self.writer;
        for owner in owners {
            warm(
                self.host,
                self.scope,
                &mut writer.fold,
                &mut writer.stats,
                owner,
            )
            .await?;
            writer
                .fold
                .touch(owner.split_once('\n').map_or(owner.as_str(), |(state, _)| state));
        }
        Ok(())
    }

    pub async fn load_owner(&mut self, owner: &str) -> Result<(), StoreError> {
        let writer = &mut *self.writer;
        warm_state(
            self.host,
            self.scope,
            &mut writer.fold,
            &mut writer.stats,
            owner,
        )
        .await
    }

    #[must_use]
    pub fn buffered(&self) -> usize {
        self.writer.buffer.len()
    }

    pub fn unride(&mut self) {
        while let Ok(riding) = self.writer.riders.try_recv() {
            let _ = riding.rode.send(false);
        }
    }

    pub fn next_event(&mut self) -> u64 {
        self.writer.fold.next_event()
    }

    pub fn collect_unreachable(&mut self, state: &str) {
        if !self.writer.collectable.contains(state) {
            self.writer.collectable.insert(state.to_owned());
        }
    }

    pub fn push(&mut self, record: Record) -> Result<(), StoreError> {
        let writer = &mut *self.writer;
        if let Record::Checkpoint { state, .. } = &record
            && !self.whole
            && !writer.bases.contains_key(state)
        {
            let base = writer
                .fold
                .checkpoint(state)
                .filter(|held| writer.tails.get(state) == Some(&held.revision))
                .cloned();
            writer.bases.insert(state.clone(), base);
        }
        writer.fold.apply(&record, None)?;
        if let Record::Checkpoint { state, .. } = &record {
            writer.buffer.retain(
                |buffered| !matches!(buffered, Record::Checkpoint { state: other, .. } if other == state),
            );
        }
        writer.buffer.push(record);
        Ok(())
    }

    pub fn unadmit(&mut self, turn: &str, seq: u64) {
        let writer = &mut *self.writer;
        writer.buffer.retain(|record| match record {
            Record::Admitted { turn: admitted, .. } => admitted != turn,
            Record::Event { seq: event, .. } => *event != seq,
            _ => true,
        });
        writer.fold.unadmit(turn, seq);
    }

    #[must_use]
    pub fn floor_pending(&self) -> bool {
        self.writer.floor.is_some()
    }

    pub async fn secure(&mut self) -> Result<(), StoreError> {
        if self.writer.floor.is_some() {
            self.commit(Point::Floor).await?;
        }
        Ok(())
    }

    pub async fn commit(&mut self, point: Point) -> Result<u64, StoreError> {
        self.committed(point, None).await.map(|(seq, _)| seq)
    }

    pub async fn commit_carrying(
        &mut self,
        point: Point,
        rider: Rider<'_>,
    ) -> Result<(u64, bool), StoreError> {
        self.committed(point, Some(rider)).await
    }

    fn committed<'b>(
        &'b mut self,
        point: Point,
        rider: Option<Rider<'b>>,
    ) -> nanocode_durability_core::StoreFuture<'b, Result<(u64, bool), StoreError>> {
        Box::pin(self.write(point, rider))
    }

    async fn write(
        &mut self,
        point: Point,
        rider: Option<Rider<'_>>,
    ) -> Result<(u64, bool), StoreError> {
        let writer = &mut *self.writer;
        let revived = writer.fold.revive(&writer.buffer);
        writer.buffer.extend(revived);
        let seq = writer.next;
        let previous = writer.fold.seq();
        let recovered = writer
            .buffer
            .iter()
            .filter_map(|record| match record {
                Record::Recovery { turn, .. } => Some(turn.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let floor = writer.floor.map(|seq| Record::Floor { seq });
        let plain = {
            let mut written = Vec::with_capacity(writer.buffer.len());
            for record in &writer.buffer {
                written.push(match record {
                    Record::Checkpoint {
                        payload,
                        revision,
                        state,
                    } => match writer.bases.get(state) {
                        Some(Some(base)) => crate::patch::diff(&base.payload, payload)?.map_or(
                            Cow::Borrowed(record),
                            |edits| {
                                Cow::Owned(Record::Patched {
                                    base: base.revision,
                                    edits,
                                    revision: *revision,
                                    state: state.clone(),
                                })
                            },
                        ),
                        _ => Cow::Borrowed(record),
                    },
                    record => Cow::Borrowed(record),
                });
            }
            crate::record::encode_all(floor.iter().chain(written.iter().map(|record| &**record)))?
        };
        let mut stream = match writer.stream.take() {
            Some(stream) if writer.stale => stream.restart(writer.history.bytes())?,
            Some(stream) => stream,
            None => Stream::new(writer.history.bytes())?,
        };
        let framed = if self.rows.is_some() && recovered.is_empty() && writer.tail_row.is_some() {
            let mut framed = Vec::with_capacity(plain.len() + 1);
            framed.push(b'\n');
            framed.extend_from_slice(&plain);
            Cow::Owned(framed)
        } else {
            Cow::Borrowed(plain.as_slice())
        };
        let mut body = stream.pack(RECORDS, &framed)?;
        let keep = writer
            .buffer
            .iter()
            .find_map(|record| match record {
                Record::Event { seq, .. } => Some(*seq),
                _ => None,
            })
            .unwrap_or(u64::MAX);
        if writer.fold.segment_row() == 0 {
            writer.fold.start_segment(seq);
        }
        let mut snapshot = None;
        let due = self.rows.is_some_and(|rows| writer.tail_rows >= rows);
        if due || writer.tail + body.len() >= writer.check {
            for owner in writer.fold.forgetting() {
                warm(
                    self.host,
                    self.scope,
                    &mut writer.fold,
                    &mut writer.stats,
                    &owner,
                )
                .await?;
            }
            for owner in writer.fold.waiting() {
                page(
                    self.host,
                    self.scope,
                    &mut writer.fold,
                    &mut writer.stats,
                    &owner,
                )
                .await?;
            }
            root(
                self.host,
                self.scope,
                &mut writer.fold,
                &mut writer.stats,
            )
            .await?;
            for (first, section) in writer.fold.unlisted() {
                let records = section_records(self.host, self.scope, &first, section).await?;
                writer.fold.paged(&first, records)?;
                writer.stats.index_loads += 1;
            }
            writer.stats.collected += writer.fold.collect(&writer.collectable) as u64;
            let level = if due {
                codec::FAST_LEVEL
            } else {
                codec::DEFAULT_LEVEL
            };
            let sections = sections(
                &mut writer.spare,
                &writer.fold,
                seq,
                level,
                self.limit,
                self.ratio,
            )?;
            let index = index(&mut writer.spare, &writer.fold, &sections, level, self.limit)?;
            let head = index.next;
            for turn in &recovered {
                writer.fold.recovered_at(turn, head);
            }
            writer.fold.committed(head);
            let packed = packed(
                &mut writer.spare,
                &writer.fold,
                keep,
                level,
                index.root,
            );
            writer.fold.committed(previous);
            let packed = packed?;
            writer.state = packed.plain;
            let rewrite = index.bytes
                + usize::try_from(
                    (sections.bytes as u128 * u128::from(sections.rewritten))
                        .checked_div(u128::from(sections.plain))
                        .unwrap_or(0),
                )
                .map_err(|error| {
                    StoreError::NotCommitted(format!("turn log rewrite size: {error}"))
                })?;
            let size = packed.packed.len() + rewrite;
            let tail = writer.tail + body.len();
            if due || (writer.snapshot + tail > self.ratio * size && tail >= self.ratio * rewrite) {
                snapshot = Some(Partitioned {
                    head: packed,
                    index,
                    rewrite,
                    sections,
                    seq: head,
                });
            } else {
                let packed = size;
                let due = (self.ratio * packed).saturating_sub(writer.snapshot);
                let projected = match writer.tried {
                    Some((before, packed_before)) if tail > before && packed > packed_before => {
                        let gap = (tail - before) as u128;
                        let grown = (self.ratio * (packed - packed_before)) as u128;
                        if grown >= gap {
                            self.ratio * tail
                        } else {
                            let reach = gap * due as u128;
                            let rise = grown * tail as u128;
                            usize::try_from(reach.saturating_sub(rise) / (gap - grown))
                                .unwrap_or(usize::MAX)
                        }
                    }
                    _ => due,
                };
                writer.tried = Some((tail, packed));
                writer.check = writer
                    .snapshot
                    .max(due)
                    .max(projected)
                    .max(self.ratio * rewrite);
            }
        }
        let mut boundary = None;
        if snapshot.is_none() && writer.fold.hot_bytes() > writer.state {
            writer.state = measured(&writer.fold, keep)?;
            if writer.fold.hot_bytes() > writer.state {
                boundary = Some(codec::pack(&mut writer.spare, BOUNDARY, &plain, &[])?);
            }
        }
        let seq = snapshot.as_ref().map_or(seq, |part| part.seq);
        let body_len = body.len();
        let written = match (&mut snapshot, &mut boundary) {
            (Some(part), _) => std::mem::take(&mut part.head.packed),
            (None, Some(boundary)) => std::mem::take(boundary),
            (None, None) => std::mem::take(&mut body),
        };
        let written_len = written.len();
        let stored = stored(seq, written, self.limit)?;
        let bytes = stored.iter().map(|(_, body)| body.len()).sum::<usize>();
        let archived = match (&snapshot, &writer.previous) {
            (Some(_), Some(previous)) => previous
                .archive(&mut writer.spare, self.limit)?
                .map(|rows| (rows, previous.seq, previous.parts)),
            _ => None,
        };
        let extension = match (&snapshot, &boundary, &writer.tail_row) {
            (None, None, Some(tail))
                if self.rows.is_some()
                    && recovered.is_empty()
                    && stored.len() == 1
                    && self
                        .limit
                        .is_none_or(|limit| tail.bytes - SEAL + body_len - 1 <= limit) =>
            {
                Some((
                    Extension {
                        cut: SEAL,
                        row: tail.seq,
                        seq,
                        tail: &stored[0].1[1..],
                    },
                    tail.bytes - SEAL + body_len - 1,
                ))
            }
            _ => None,
        };
        let (extension, merged) = extension.unzip();
        let (sectioned, superseded) = match &mut snapshot {
            Some(part) => {
                let mut rows = std::mem::take(&mut part.sections.rows);
                rows.append(&mut part.index.rows);
                (rows, std::mem::take(&mut part.index.superseded))
            }
            None => (Vec::new(), Vec::new()),
        };
        let sectioned_rows = sectioned.len() as u64;
        let appended = if sectioned.is_empty() {
            Cow::Borrowed(stored.as_slice())
        } else {
            Cow::Owned(
                sectioned
                    .into_iter()
                    .chain(stored.iter().cloned())
                    .collect::<Vec<_>>(),
            )
        };
        let (kept, dropped) = match &archived {
            Some((rows, from, parts)) => (
                rows.as_slice(),
                from + rows.len() as u64..from + *parts as u64,
            ),
            None => (&[][..], 0..0),
        };
        let compaction = Compaction {
            archived: kept,
            dropped,
            superseded: &superseded,
        };
        let mut riding = Vec::new();
        while let Ok(ride) = self.writer.riders.try_recv() {
            riding.push(ride);
        }
        let host = rider
            .map(|rider| rider.host)
            .or_else(|| riding.first().map(|ride| ride.host));
        let (riding, apart): (Vec<_>, Vec<_>) =
            riding.into_iter().partition(|ride| Some(ride.host) == host);
        for ride in apart {
            let _ = ride.rode.send(false);
        }
        let (held, rodes): (Vec<_>, Vec<_>) = riding
            .into_iter()
            .map(|ride| (ride.statements, ride.rode))
            .unzip();
        let mut statements = rider.map_or_else(Vec::new, |rider| rider.statements.to_vec());
        statements.extend(held.iter().flatten().map(|(sql, params)| SqlStatement {
            sql,
            params: params.iter().map(String::as_str).collect(),
        }));
        let explicit = rider.is_some();
        let rider = host.map(|host| Rider {
            host,
            statements: &statements,
        });
        let answer = |rodes: Vec<oneshot::Sender<bool>>, rode: bool| {
            for sender in rodes {
                let _ = sender.send(rode);
            }
        };
        let (carried, extended) = {
            let carrying = rider.and_then(|rider| {
                let write = match extension {
                    Some(extension) => Write::Extend(extension),
                    None => Write::Append {
                        rows: &appended,
                        compaction: compaction.clone(),
                    },
                };
                self.host.carry(self.scope, write, rider)
            });
            let carried = carrying.is_some();
            let extended = match carrying {
                Some(carrying) => {
                    let done = carrying.await;
                    answer(rodes, done.is_ok());
                    done?;
                    extension.and(merged)
                }
                None => {
                    answer(rodes, false);
                    let extending =
                        extension.and_then(|extension| self.host.extend(self.scope, extension));
                    let extended = extending.is_some().then_some(merged).flatten();
                    match extending {
                        Some(extending) => extending.await?,
                        None if archived.is_some() || !superseded.is_empty() => {
                            self.host
                                .append_compacting(self.scope, &appended, compaction)
                                .await?;
                        }
                        None => self.host.append(self.scope, &appended).await?,
                    }
                    extended
                }
            };
            (carried, extended)
        };
        drop(appended);
        drop(statements);
        drop(held);
        let added = if extended.is_some() {
            body_len - 1 - SEAL
        } else {
            bytes
        };
        let writer = &mut *self.writer;
        writer.tail_row = match (extended, &snapshot, &boundary) {
            (Some(merged), _, _) => Some(TailRow { bytes: merged, seq }),
            (None, None, None) if recovered.is_empty() && stored.len() == 1 => {
                Some(TailRow { bytes, seq })
            }
            _ => None,
        };
        writer.next = seq + stored.len() as u64;
        writer.fold.committed(seq);
        for turn in &recovered {
            writer.fold.recovered_at(turn, seq);
        }
        if snapshot.is_some() {
            writer.tails.clear();
        } else {
            for record in &writer.buffer {
                if let Record::Checkpoint {
                    revision, state, ..
                } = record
                {
                    writer.tails.insert(state.clone(), *revision);
                }
            }
        }
        writer.bases.clear();
        writer.buffer.clear();
        writer.floor = None;
        if let Some(keep) = self.retain {
            writer.stats.retired += writer.fold.retire(keep) as u64;
        }
        writer.stats.parts += stored.len() as u64 - 1;
        writer.stats.plain_bytes += plain.len() as u64;
        match snapshot {
            Some(part) => {
                let packed = part.head;
                let rewritten = part.sections.bytes + part.index.bytes;
                writer
                    .fold
                    .sectioned(
                        part.sections.entries,
                        part.index.pages,
                        part.index.groups,
                        part.index.root,
                    );
                writer.fold.evict();
                writer.stats.sections += sectioned_rows;
                writer.stats.snapshot_bytes += rewritten as u64;
                writer.fold.seal(seq, keep);
                writer.history.clear();
                writer.history.extend(packed.suffix());
                writer.previous = packed.trimmed().then(|| Previous {
                    bytes,
                    parts: stored.len(),
                    seq,
                    suffix: String::from_utf8(packed.suffix().to_vec())
                        .expect("turn log records encode as UTF-8"),
                });
                writer.snapshot = bytes + part.rewrite;
                writer.check = bytes + part.rewrite;
                writer.tail = 0;
                writer.tail_rows = stored.len() - 1;
                writer.tried = None;
                writer.stream = Some(stream);
                writer.stale = true;
                writer.stats.snapshots += 1;
                writer.stats.snapshot_bytes += written_len as u64;
            }
            None => {
                if boundary.is_some() {
                    writer.fold.seal(seq, keep);
                    writer.history.clear();
                    writer.stats.boundaries += 1;
                }
                writer.stream = Some(stream);
                writer.stale = boundary.is_some();
                writer
                    .history
                    .extend(if boundary.is_some() { &plain } else { &framed });
                writer.tail += added;
                writer.tail_rows += stored.len();
                writer.stats.row_bytes += added as u64;
            }
        }
        writer.fold.release();
        match point {
            Point::Admission => writer.stats.admissions += 1,
            Point::Call => writer.stats.calls += 1,
            Point::Cancel => writer.stats.cancels += 1,
            Point::Finish => writer.stats.finishes += 1,
            Point::Floor => writer.stats.floors += 1,
            Point::Progress => writer.stats.progress += 1,
            Point::Recovery => writer.stats.recoveries += 1,
        }
        Ok((seq, carried && explicit))
    }
}
