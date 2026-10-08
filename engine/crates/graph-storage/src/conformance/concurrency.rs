//! Concurrency cases of the GS-001 suite.

use super::*;
use crate::types::FileVersionKey;

/// Twenty concurrent upserts of the same keys converge on one id per key, and a lookup by
/// key returns those ids. The second tenant gets its own rows.
pub async fn upsert_file_versions_is_idempotent_under_concurrency(
    h: &dyn Harness,
) -> Result<(), String> {
    let f = h.fresh().await.map_err(show)?;
    let inputs = file_version_inputs(&fixture_graph());
    if inputs.is_empty() {
        return Err("the fixture produced no file versions".to_owned());
    }

    let calls = (0..20).map(|_| f.store.upsert_file_versions(&f.scope, &inputs));
    let results = futures::future::join_all(calls).await;
    let first = results
        .first()
        .ok_or("no upsert was started")?
        .as_ref()
        .map_err(|e| e.to_string())?;
    if first.len() != inputs.len() {
        return Err(format!(
            "upsert returned {} refs for {} inputs",
            first.len(),
            inputs.len()
        ));
    }
    for (index, result) in results.iter().enumerate() {
        let refs = result.as_ref().map_err(|e| e.to_string())?;
        for (want, got) in first.iter().zip(refs) {
            if want.id != got.id {
                return Err(format!(
                    "call {index} allocated id {} for {} where call 0 used {}",
                    got.id, got.path, want.id
                ));
            }
        }
    }

    let keys: Vec<FileVersionKey> = inputs.iter().map(FileVersionInput::key).collect();
    let found = f
        .store
        .lookup_file_versions(&f.scope, &keys)
        .await
        .map_err(show)?;
    if found.len() != keys.len() {
        return Err(format!(
            "lookup returned {} results for {} keys",
            found.len(),
            keys.len()
        ));
    }
    for (want, got) in first.iter().zip(&found) {
        match got {
            Some(ref_id) if ref_id.id == want.id => {}
            Some(ref_id) => {
                return Err(format!(
                    "lookup returned id {} for {} where upsert used {}",
                    ref_id.id, ref_id.path, want.id
                ))
            }
            None => return Err(format!("{} is missing after upsert", want.path)),
        }
    }

    // Unknown keys resolve to `None`, not to an error.
    let mut unknown = inputs[0].key();
    unknown.content_hash = [0xab; 32];
    let missing = f
        .store
        .lookup_file_versions(&f.scope, std::slice::from_ref(&unknown))
        .await
        .map_err(show)?;
    if !matches!(missing.as_slice(), [None]) {
        return Err("an unknown key resolved to a file version".to_owned());
    }

    // Another tenant upserting the same content gets its own rows.
    let foreign = f
        .store
        .upsert_file_versions(&f.foreign, &inputs)
        .await
        .map_err(show)?;
    for (want, got) in first.iter().zip(&foreign) {
        if want.id == got.id {
            return Err(format!(
                "two tenants share file version {} for {}",
                want.id, want.path
            ));
        }
    }
    let foreign_lookup = f
        .store
        .lookup_file_versions(&f.foreign, &keys)
        .await
        .map_err(show)?;
    for (want, got) in foreign.iter().zip(&foreign_lookup) {
        match got {
            Some(ref_id) if ref_id.id == want.id => {}
            _ => return Err("the foreign lookup did not return the foreign ids".to_owned()),
        }
    }
    let original = f
        .store
        .lookup_file_versions(&f.scope, &keys)
        .await
        .map_err(show)?;
    for (want, got) in first.iter().zip(&original) {
        match got {
            Some(ref_id) if ref_id.id == want.id => {}
            _ => return Err("the first tenant's rows changed".to_owned()),
        }
    }
    Ok(())
}
