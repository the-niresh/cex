//! Snapshots on disk.
//!
//! Writes are atomic: the bytes go to a `.tmp` file which is flushed and then
//! **renamed** into place. `rename` within a directory is atomic on every
//! filesystem we care about, so a crash mid-write leaves the previous snapshot
//! whole rather than producing a truncated one that looks loadable.
//!
//! Reads are forgiving in one specific way: a corrupt newest snapshot falls back
//! to the one before it. Losing a snapshot costs a longer replay; refusing to
//! boot costs an outage.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cex_core::state::Snapshot;
use tracing::warn;

use crate::stream_id::StreamId;

const EXT: &str = "snapshot";
const TMP_EXT: &str = "snapshot.tmp";

pub struct SnapshotStore {
    dir: PathBuf,
    /// How many snapshots to retain. Older ones are deleted by [`prune`].
    keep: usize,
}

impl SnapshotStore {
    pub fn new(dir: impl Into<PathBuf>, keep: usize) -> Self {
        SnapshotStore {
            dir: dir.into(),
            keep: keep.max(1),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Write a snapshot and return the path it landed at.
    pub fn save(&self, snap: &Snapshot) -> Result<PathBuf> {
        fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating snapshot dir {}", self.dir.display()))?;

        let bytes = snap
            .encode()
            .map_err(|e| anyhow::anyhow!("encoding snapshot: {e}"))?;

        let final_path = self.dir.join(format!("{}.{EXT}", snap.last_stream_id));
        let tmp_path = self.dir.join(format!("{}.{TMP_EXT}", snap.last_stream_id));

        {
            let mut f = File::create(&tmp_path)
                .with_context(|| format!("creating {}", tmp_path.display()))?;
            f.write_all(&bytes)?;
            // Without this the rename can land before the data does, and a power
            // loss leaves a correctly-named but empty file.
            f.sync_all()?;
        }

        fs::rename(&tmp_path, &final_path).with_context(|| {
            format!(
                "renaming {} to {}",
                tmp_path.display(),
                final_path.display()
            )
        })?;

        Ok(final_path)
    }

    /// Every valid-looking snapshot file, newest first.
    ///
    /// Only files named `<stream-id>.snapshot` count. Anything else in the
    /// directory — a leftover `.tmp`, a README, a file with an unparseable name —
    /// is ignored rather than guessed at.
    pub fn list(&self) -> Result<Vec<(StreamId, PathBuf)>> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).context("reading snapshot dir"),
        };

        let mut found: Vec<(StreamId, PathBuf)> = Vec::new();
        for entry in entries {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // `strip_suffix` on the full extension also rejects `.snapshot.tmp`,
            // because that name does not end in exactly `.snapshot`.
            let Some(stem) = name.strip_suffix(&format!(".{EXT}")) else {
                continue;
            };
            let Some(id) = StreamId::parse(stem) else {
                continue;
            };
            found.push((id, path));
        }

        // Newest first, compared numerically — see `stream_id`.
        found.sort_by_key(|(id, _)| std::cmp::Reverse(*id));
        Ok(found)
    }

    /// The newest snapshot that actually decodes.
    pub fn load_latest(&self) -> Result<Option<Snapshot>> {
        for (id, path) in self.list()? {
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "unreadable snapshot, skipping");
                    continue;
                }
            };
            match Snapshot::decode(&bytes) {
                Ok(snap) => return Ok(Some(snap)),
                Err(e) => {
                    warn!(
                        id = %id,
                        error = %e,
                        "corrupt snapshot, falling back to an older one"
                    );
                }
            }
        }
        Ok(None)
    }

    /// Where replay should resume from. `ZERO` means "no usable snapshot, replay
    /// the whole log" — correct, just slower.
    pub fn resume_position(&self) -> Result<StreamId> {
        Ok(match self.load_latest()? {
            Some(snap) => StreamId::parse(&snap.last_stream_id).unwrap_or(StreamId::ZERO),
            None => StreamId::ZERO,
        })
    }

    /// Delete all but the newest `keep` snapshots. Returns how many were removed.
    pub fn prune(&self) -> Result<usize> {
        let all = self.list()?;
        let mut removed = 0;
        for (_, path) in all.into_iter().skip(self.keep) {
            match fs::remove_file(&path) {
                Ok(()) => removed += 1,
                Err(e) => warn!(path = %path.display(), error = %e, "could not prune snapshot"),
            }
        }
        Ok(removed)
    }
}
