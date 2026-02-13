use std::sync::Arc;

use async_trait::async_trait;
use chrono::prelude::*;
use std::time::{Duration, Instant};
use tokio::sync::watch;

use garage_util::background::*;
use garage_util::error::Error;
use garage_util::persister::PersisterShared;
use garage_util::time::*;

use garage_table::EmptyKey;

use crate::bucket_table::*;
use crate::s3::lifecycle_worker::{midnight_ts, today};
use crate::s3::object_table::*;

use crate::garage::Garage;

mod v090 {
	use serde::{Deserialize, Serialize};

	#[derive(Serialize, Deserialize, Default, Clone)]
	pub struct VersionPruningWorkerPersisted {
		pub last_completed: Option<String>,
	}

	impl garage_util::migrate::InitialFormat for VersionPruningWorkerPersisted {
		const VERSION_MARKER: &'static [u8] = b"G09vpwp";
	}
}

pub use v090::*;

pub struct VersionPruningWorker {
	garage: Arc<Garage>,
	state: State,
	persister: PersisterShared<VersionPruningWorkerPersisted>,
}

#[expect(clippy::large_enum_variant)]
enum State {
	Completed(NaiveDate),
	Running {
		date: NaiveDate,
		pos: Vec<u8>,
		counter: usize,
		versions_pruned: usize,
		last_bucket: Option<Bucket>,
	},
}

pub fn register_bg_vars(
	persister: &PersisterShared<VersionPruningWorkerPersisted>,
	vars: &mut vars::BgVars,
) {
	vars.register_ro(persister, "version-pruning-last-completed", |p| {
		p.get_with(|x| x.last_completed.clone().unwrap_or("never".to_string()))
	});
}

impl VersionPruningWorker {
	pub fn new(
		garage: Arc<Garage>,
		persister: PersisterShared<VersionPruningWorkerPersisted>,
	) -> Self {
		let today = today(garage.config.use_local_tz);
		let last_completed = persister.get_with(|x| {
			x.last_completed
				.as_deref()
				.and_then(|x| x.parse::<NaiveDate>().ok())
		});
		let state = match last_completed {
			Some(d) if d >= today => State::Completed(d),
			_ => State::start(today),
		};
		Self {
			garage,
			state,
			persister,
		}
	}
}

impl State {
	fn start(date: NaiveDate) -> Self {
		info!("Starting version pruning worker for {}", date);
		State::Running {
			date,
			pos: vec![],
			counter: 0,
			versions_pruned: 0,
			last_bucket: None,
		}
	}
}

#[async_trait]
impl Worker for VersionPruningWorker {
	fn name(&self) -> String {
		"version pruning worker".to_string()
	}

	fn status(&self) -> WorkerStatus {
		match &self.state {
			State::Completed(d) => WorkerStatus {
				freeform: vec![format!("Last completed: {}", d)],
				..Default::default()
			},
			State::Running {
				date,
				counter,
				versions_pruned,
				..
			} => {
				let n_objects = self.garage.object_table.data.store.approximate_len().ok();
				let progress = match n_objects {
					Some(total) if total > 0 => format!(
						"~{:.2}%",
						100. * std::cmp::min(*counter, total) as f32 / total as f32
					),
					_ => "...".to_string(),
				};
				WorkerStatus {
					progress: Some(progress),
					freeform: vec![
						format!("Started: {}", date),
						format!("Versions pruned: {}", versions_pruned),
					],
					..Default::default()
				}
			}
		}
	}

	async fn work(&mut self, _must_exit: &mut watch::Receiver<bool>) -> Result<WorkerState, Error> {
		match &mut self.state {
			State::Completed(_) => Ok(WorkerState::Idle),
			State::Running {
				date,
				counter,
				versions_pruned,
				pos,
				last_bucket,
			} => {
				// Process a batch of 100 items before yielding to bg task scheduler
				for _ in 0..100 {
					let (object_bytes, next_pos) = match self
						.garage
						.object_table
						.data
						.store
						.get_gt(&mut *pos)?
					{
						None => {
							info!(
								"Version pruning worker finished for {}, versions pruned: {}",
								date, *versions_pruned
							);
							self.persister
								.set_with(|x| x.last_completed = Some(date.to_string()))?;
							self.state = State::Completed(*date);
							return Ok(WorkerState::Idle);
						}
						Some((k, v)) => (v, k),
					};

					let object = self.garage.object_table.data.decode_entry(&object_bytes)?;
					let skip = process_object(
						&self.garage,
						&object,
						versions_pruned,
						last_bucket,
					)
					.await?;

					*counter += 1;
					if skip == Skip::SkipBucket {
						let bucket_id_len = object.bucket_id.as_slice().len();
						assert_eq!(
							next_pos.get(..bucket_id_len),
							Some(object.bucket_id.as_slice())
						);
						let last_bucket_pos = [&next_pos[..bucket_id_len], &[0xFFu8][..]].concat();
						*pos = std::cmp::max(next_pos, last_bucket_pos);
					} else {
						*pos = next_pos;
					}
				}

				Ok(WorkerState::Busy)
			}
		}
	}

	async fn wait_for_work(&mut self) -> WorkerState {
		match &self.state {
			State::Completed(d) => {
				let use_local_tz = self.garage.config.use_local_tz;
				let next_day = d.succ_opt().expect("no next day");
				let next_start = midnight_ts(next_day, use_local_tz);
				loop {
					let now = now_msec();
					if now < next_start {
						tokio::time::sleep_until(
							(Instant::now() + Duration::from_millis(next_start - now)).into(),
						)
						.await;
					} else {
						break;
					}
				}
				self.state = State::start(std::cmp::max(next_day, today(use_local_tz)));
			}
			State::Running { .. } => (),
		}
		WorkerState::Busy
	}
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Skip {
	SkipBucket,
	NextObject,
}

async fn process_object(
	garage: &Arc<Garage>,
	object: &Object,
	versions_pruned: &mut usize,
	last_bucket: &mut Option<Bucket>,
) -> Result<Skip, Error> {
	// Don't prune versions of locked objects
	if object.is_locked() {
		return Ok(Skip::NextObject);
	}

	// Only process objects that have more than one complete version
	let complete_versions: Vec<_> = object
		.versions()
		.iter()
		.filter(|v| v.is_complete())
		.collect();

	if complete_versions.len() <= 1 {
		return Ok(Skip::NextObject);
	}

	// Look up bucket to check versioning state
	let bucket = match last_bucket.take() {
		Some(b) if b.id == object.bucket_id => b,
		_ => match garage
			.bucket_table
			.get(&EmptyKey, &object.bucket_id)
			.await?
		{
			Some(b) => b,
			None => {
				warn!(
					"Version pruning worker: object in non-existent bucket {:?}",
					object.bucket_id
				);
				return Ok(Skip::SkipBucket);
			}
		},
	};

	let versioning = bucket
		.state
		.as_option()
		.map(|s| s.versioning.get().clone())
		.unwrap_or(BucketVersioning::Unversioned);

	// For versioned/suspended buckets, don't prune
	if versioning != BucketVersioning::Unversioned {
		*last_bucket = Some(bucket);
		return Ok(Skip::NextObject);
	}

	*last_bucket = Some(bucket);

	// For non-versioned buckets: mark all complete versions except the latest as Aborted
	let latest_complete = complete_versions.last().unwrap();

	let db = garage.object_table.data.store.db();

	for version in complete_versions.iter() {
		if version.uuid == latest_complete.uuid {
			continue;
		}

		let aborted = ObjectVersion {
			uuid: version.uuid,
			timestamp: version.timestamp,
			state: ObjectVersionState::Aborted,
		};
		let obj = Object::new(object.bucket_id, object.key.clone(), vec![aborted]);

		// Use queue_insert for consistency with lifecycle worker pattern
		let res = db.transaction(|tx| garage.object_table.queue_insert(tx, &obj));
		if let Err(e) = res {
			warn!(
				"Version pruning worker: error pruning version {}: {:?}",
				hex::encode(version.uuid),
				e
			);
		} else {
			*versions_pruned += 1;
		}
	}

	Ok(Skip::NextObject)
}
