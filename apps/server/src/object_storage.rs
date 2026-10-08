//! Object storage (S3) for attachments and backups, as saved by the root user in Admin → Storage: builds the
//! bucket from the saved settings, applies them while the server runs, moves blobs in the background and makes
//! the automatic backups.

use std::time::Duration;

use orbit_platform::{
    BackupService, Database, ObjectStorage, ObjectStorageState, S3Bucket, S3Config,
    TieredBlobStore, TimestampMillis,
};
use tokio_util::sync::CancellationToken;

use crate::repositories::instance_settings::{
    BackupSchedule, InstanceSettingsRepository, S3Settings, StorageSettings,
};
use crate::secret_box::decrypt_secret;

const HOUR_MILLIS: i64 = 60 * 60 * 1000;
/// The mover also runs this often without a settings change, to retry after an error.
const MOVE_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// The backup job runs every hour; a backup is due a little before its period ends, so a job that runs a few
/// minutes early does not skip a period.
const SCHEDULE_SLACK_MILLIS: i64 = 10 * 60 * 1000;

/// `secret_access_key` in plain text, for a bucket that is not saved yet.
pub fn bucket(s3: &S3Settings, secret_access_key: String) -> Result<S3Bucket, String> {
    S3Bucket::new(S3Config {
        endpoint: s3.endpoint.clone(),
        region: s3.region.clone(),
        bucket: s3.bucket.clone(),
        prefix: s3.prefix.clone(),
        access_key_id: s3.access_key_id.clone(),
        secret_access_key,
        path_style: s3.path_style,
    })
    .map_err(|error| error.to_string())
}

/// The saved bucket, with its secret decrypted under the app key.
pub fn saved_bucket(s3: &S3Settings, app_key: Option<&[u8; 32]>) -> Result<S3Bucket, String> {
    let key = app_key.ok_or("the server has no app key to read the S3 secret with")?;
    let secret = decrypt_secret(key, &s3.secret_access_key)
        .map_err(|()| "the S3 secret cannot be decrypted with this server's app key".to_owned())?;
    bucket(s3, secret)
}

/// Makes the running server use `settings`.
pub fn apply(
    storage: &ObjectStorage,
    settings: &StorageSettings,
    app_key: Option<&[u8; 32]>,
) -> Result<(), String> {
    let bucket = settings
        .s3
        .as_ref()
        .map(|s3| saved_bucket(s3, app_key))
        .transpose()?;
    storage.configure(ObjectStorageState {
        bucket,
        attachments: settings.attachments_in_s3,
        backups: settings.backups_in_s3,
    });
    Ok(())
}

/// Moves blobs to the active tier after every settings change, and every ten minutes to retry after errors.
pub async fn run_mover(
    store: TieredBlobStore,
    storage: ObjectStorage,
    shutdown: CancellationToken,
) -> Result<(), String> {
    loop {
        match store.move_blobs().await {
            Ok(0) => {}
            Ok(moved) => tracing::info!(moved, "moved attachment blobs between disk and S3"),
            Err(error) => tracing::warn!(%error, "moving attachment blobs failed; retrying later"),
        }
        tokio::select! {
            () = shutdown.cancelled() => return Ok(()),
            () = storage.changed() => {}
            () = tokio::time::sleep(MOVE_INTERVAL) => {}
        }
    }
}

/// Runs every hour: creates a backup when the newest one is older than the saved schedule's period.
pub async fn run_scheduled_backup(
    database: &Database,
    backups: &BackupService,
) -> Result<(), String> {
    let schedule = InstanceSettingsRepository::new(database.clone())
        .storage()
        .await
        .map_err(|error| error.to_string())?
        .backup_schedule;
    let period = match schedule {
        BackupSchedule::Off => return Ok(()),
        BackupSchedule::Hourly => HOUR_MILLIS,
        BackupSchedule::Daily => 24 * HOUR_MILLIS,
    };
    let newest = backups
        .list()
        .await
        .map_err(|error| error.to_string())?
        .first()
        .map(|snapshot| snapshot.manifest.created_at);
    let now = TimestampMillis::now().as_millis();
    if newest.is_some_and(|created_at| now - created_at < period - SCHEDULE_SLACK_MILLIS) {
        return Ok(());
    }
    backups
        .create(database)
        .await
        .map(drop)
        .map_err(|error| error.to_string())
}

/// The bucket for `orbit backup …` from `ORBIT_BACKUP_S3_ENDPOINT`, `_BUCKET`, `_ACCESS_KEY_ID`,
/// `_SECRET_ACCESS_KEY` and optional `_REGION`, `_PREFIX`, `_PATH_STYLE=true`. The command line does not read the
/// saved settings: the database is locked while the server runs, and after a lost server only the bucket is left.
pub fn from_env() -> Result<ObjectStorage, String> {
    let storage = ObjectStorage::default();
    let variable = |name: &str| std::env::var(format!("ORBIT_BACKUP_S3_{name}")).ok();
    let Some(endpoint) = variable("ENDPOINT") else {
        return Ok(storage);
    };
    let required = |name: &str| variable(name).ok_or(format!("ORBIT_BACKUP_S3_{name} is not set"));
    let bucket = S3Bucket::new(S3Config {
        endpoint,
        region: variable("REGION").unwrap_or_default(),
        bucket: required("BUCKET")?,
        prefix: variable("PREFIX").unwrap_or_default(),
        access_key_id: required("ACCESS_KEY_ID")?,
        secret_access_key: required("SECRET_ACCESS_KEY")?,
        path_style: variable("PATH_STYLE").is_some_and(|value| value == "true" || value == "1"),
    })
    .map_err(|error| error.to_string())?;
    storage.configure(ObjectStorageState {
        bucket: Some(bucket),
        attachments: false,
        backups: true,
    });
    Ok(storage)
}
