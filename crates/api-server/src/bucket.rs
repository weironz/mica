//! Make sure the configured bucket exists, using nothing but the S3 API.
//!
//! This replaced a `rustfs-init` service in both compose files that ran
//! `mkdir -p /data/<bucket>` in the RustFS container. That worked, and only
//! worked, because RustFS happens to be filesystem-backed: it assumed a bucket
//! IS a directory, that the image ships a `rustfs` user to chown to, and that
//! there is a container to exec in at all. Point `S3_ENDPOINT` at AWS or
//! Alibaba OSS and none of those three hold.
//!
//! The failure it exists to prevent is the nastiest shape available: with no
//! bucket, every container reports healthy, `/api/ready` returns 200, and only
//! the first person to paste an image finds out — their upload 404s.
//!
//! Policy lives in `mica_infra::storage` as pure functions (`classify_head`,
//! `classify_create`) so each branch is unit-tested without an object store.
//! This module is only the IO around them. The reasoning behind the branches,
//! and the survey of how Gitea / Nextcloud / AFFiNE handle this, is in
//! `docs/bucket-provisioning-plan.md`.

use mica_infra::{classify_create, classify_head, BucketProbe, CreateOutcome, S3Config};
use tracing::{error, info, warn};

/// Probe for the bucket and create it if it is provably absent.
///
/// **Never fails startup**, deliberately. Mica already treats an unconfigured
/// object store as "file features off, server still runs"
/// (`S3Config::from_env` returns `None`), and turning a recoverable
/// misconfiguration into total downtime is a bad trade for a notes app whose
/// text half works fine without attachments. Every path that cannot be fixed
/// from here logs at ERROR with what to do instead.
pub async fn ensure_bucket(storage: &S3Config) {
    let http = reqwest::Client::new();

    let probe = match http.head(storage.presign_head_bucket()).send().await {
        Ok(response) => classify_head(response.status().as_u16()),
        // A transport error is not evidence of absence — the store may simply
        // not be up yet while this container is. Say so and move on rather than
        // creating anything.
        Err(err) => {
            warn!("could not probe the object-store bucket ({err}); assuming it exists");
            return;
        }
    };

    match probe {
        BucketProbe::Exists => return,
        BucketProbe::Unknown => {
            // Almost always credentials scoped to one bucket: they can read and
            // write objects but not introspect the bucket. Creating on this
            // signal is the Nextcloud bug (nextcloud/server#36427).
            info!("object-store bucket could not be introspected; assuming it exists");
            return;
        }
        BucketProbe::Missing => {}
    }

    let mut request = http.put(storage.presign_create_bucket());
    if let Some(body) = storage.create_bucket_body() {
        request = request.body(body);
    }

    let response = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            error!("object-store bucket is missing and creating it failed: {err}");
            return;
        }
    };

    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();

    match classify_create(status, &body) {
        CreateOutcome::Created => info!("created the object-store bucket"),
        CreateOutcome::AlreadyOurs => info!("object-store bucket already existed"),
        CreateOutcome::Denied => error!(
            "the object-store bucket does not exist and these credentials may not create it. \
             Create it yourself, or use credentials that can — until then every file upload \
             will fail."
        ),
        CreateOutcome::NameTaken => error!(
            "the object-store bucket name is already taken by another account (S3 bucket names \
             are globally unique on AWS). Pick a different S3_BUCKET — every file upload will \
             fail until you do."
        ),
        CreateOutcome::Failed { status } => error!(
            "the object-store bucket does not exist and creating it returned {status}: {body}"
        ),
    }
}
