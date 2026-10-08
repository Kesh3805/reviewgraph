#![allow(dead_code, clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! A small in-memory emulation of the Qdrant REST endpoints the client uses, served by
//! wiremock. Filters are evaluated like Qdrant does (must / should / must_not, match value,
//! match any over scalars and arrays, range, has_id); search is brute-force cosine.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

#[derive(Debug, Default, Clone)]
pub struct Collection {
    pub dims: u64,
    pub points: BTreeMap<String, (Vec<f32>, Map<String, Value>)>,
    pub indexes: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub struct State {
    pub collections: HashMap<String, Collection>,
    /// `(method, path, body)` of every request.
    pub log: Vec<(String, String, Value)>,
}

#[derive(Debug, Clone, Default)]
pub struct FakeQdrant {
    pub state: Arc<Mutex<State>>,
    /// Respond 503 to the next N requests.
    pub fail_next: Arc<AtomicUsize>,
    /// Search ignores filters (to emulate a server returning foreign points).
    pub ignore_search_filter: Arc<AtomicBool>,
    /// When set to `n`, every request after the n-th fails with 400 (no retry).
    pub fail_after: Arc<Mutex<Option<usize>>>,
}

fn ok(result: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"result": result, "status": "ok", "time": 0.0}))
}

fn err(code: u16, msg: &str) -> ResponseTemplate {
    ResponseTemplate::new(code).set_body_json(json!({"status": {"error": msg}, "time": 0.0}))
}

fn value_matches(stored: Option<&Value>, wanted: &Value) -> bool {
    match stored {
        Some(Value::Array(items)) => items.iter().any(|i| i == wanted),
        Some(v) => v == wanted,
        None => false,
    }
}

fn cond_matches(cond: &Value, id: &str, payload: &Map<String, Value>) -> bool {
    if let Some(ids) = cond.get("has_id").and_then(Value::as_array) {
        return ids.iter().any(|i| i.as_str() == Some(id));
    }
    let key = cond["key"].as_str().unwrap_or_default();
    let stored = payload.get(key);
    if let Some(m) = cond.get("match") {
        if let Some(v) = m.get("value") {
            return value_matches(stored, v);
        }
        if let Some(any) = m.get("any").and_then(Value::as_array) {
            return any.iter().any(|v| value_matches(stored, v));
        }
    }
    if let Some(r) = cond.get("range") {
        let Some(x) = stored.and_then(Value::as_f64) else {
            return false;
        };
        let gte = r.get("gte").and_then(Value::as_f64).is_none_or(|g| x >= g);
        let lte = r.get("lte").and_then(Value::as_f64).is_none_or(|l| x <= l);
        return gte && lte;
    }
    false
}

pub fn filter_matches(filter: &Value, id: &str, payload: &Map<String, Value>) -> bool {
    let list = |k: &str| {
        filter
            .get(k)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let must = list("must");
    let should = list("should");
    let must_not = list("must_not");
    must.iter().all(|c| cond_matches(c, id, payload))
        && (should.is_empty() || should.iter().any(|c| cond_matches(c, id, payload)))
        && !must_not.iter().any(|c| cond_matches(c, id, payload))
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    let na: f64 = a.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

fn select_payload(payload: &Map<String, Value>, with: &Value) -> Value {
    match with {
        Value::Bool(true) => Value::Object(payload.clone()),
        Value::Object(o) => {
            let include: Vec<&str> = o
                .get("include")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            Value::Object(
                payload
                    .iter()
                    .filter(|(k, _)| include.contains(&k.as_str()))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )
        }
        _ => Value::Null,
    }
}

impl FakeQdrant {
    /// Starts a mock server answering every request with this emulator.
    pub async fn start() -> (MockServer, FakeQdrant) {
        let server = MockServer::start().await;
        let fake = FakeQdrant::default();
        Mock::given(any())
            .respond_with(fake.clone())
            .mount(&server)
            .await;
        (server, fake)
    }

    pub fn collection(&self, name: &str) -> Option<Collection> {
        self.state.lock().unwrap().collections.get(name).cloned()
    }

    pub fn points(&self, name: &str) -> BTreeMap<String, (Vec<f32>, Map<String, Value>)> {
        self.collection(name).map(|c| c.points).unwrap_or_default()
    }

    /// Requests whose path ends with `suffix`.
    pub fn requests(&self, method: &str, suffix: &str) -> Vec<Value> {
        self.state
            .lock()
            .unwrap()
            .log
            .iter()
            .filter(|(m, p, _)| m == method && p.ends_with(suffix))
            .map(|(_, _, b)| b.clone())
            .collect()
    }

    pub fn create(&self, name: &str, dims: u64) {
        self.state.lock().unwrap().collections.insert(
            name.to_owned(),
            Collection {
                dims,
                ..Collection::default()
            },
        );
    }

    pub fn insert_raw(&self, collection: &str, id: &str, vector: Vec<f32>, payload: Value) {
        let mut s = self.state.lock().unwrap();
        let c = s.collections.get_mut(collection).expect("collection");
        let Value::Object(p) = payload else {
            panic!("payload must be an object")
        };
        c.points.insert(id.to_owned(), (vector, p));
    }

    fn handle(&self, method: &str, path: &str, body: &Value) -> ResponseTemplate {
        if let Some(n) = *self.fail_after.lock().unwrap() {
            if self.state.lock().unwrap().log.len() > n {
                return err(400, "injected failure");
            }
        }
        if self
            .fail_next
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return err(503, "unavailable");
        }
        if path == "/readyz" {
            return ResponseTemplate::new(200).set_body_string("all shards are ready");
        }
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let mut s = self.state.lock().unwrap();
        match (method, parts.as_slice()) {
            ("GET", ["collections", name]) => match s.collections.get(*name) {
                None => err(404, "Not found"),
                Some(c) => {
                    let schema: Map<String, Value> = c
                        .indexes
                        .iter()
                        .map(|f| (f.clone(), json!({"data_type": "keyword"})))
                        .collect();
                    ok(json!({
                        "status": "green",
                        "points_count": c.points.len(),
                        "config": {"params": {"vectors": {"size": c.dims, "distance": "Cosine"}}},
                        "payload_schema": schema,
                    }))
                }
            },
            ("PUT", ["collections", name]) => {
                if s.collections.contains_key(*name) {
                    return err(409, "Collection already exists");
                }
                let dims = body["vectors"]["size"].as_u64().unwrap_or(0);
                s.collections.insert(
                    (*name).to_owned(),
                    Collection {
                        dims,
                        ..Collection::default()
                    },
                );
                ok(json!(true))
            }
            ("DELETE", ["collections", name]) => {
                s.collections.remove(*name);
                ok(json!(true))
            }
            ("PUT", ["collections", name, "index"]) => {
                let Some(c) = s.collections.get_mut(*name) else {
                    return err(404, "Not found");
                };
                c.indexes
                    .insert(body["field_name"].as_str().unwrap_or_default().to_owned());
                ok(json!({"status": "completed"}))
            }
            ("PUT", ["collections", name, "points"]) => {
                let Some(c) = s.collections.get_mut(*name) else {
                    return err(404, "Not found");
                };
                for p in body["points"].as_array().cloned().unwrap_or_default() {
                    let id = p["id"].as_str().unwrap().to_owned();
                    let vector: Vec<f32> = p["vector"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|x| x.as_f64().unwrap() as f32)
                        .collect();
                    if vector.len() as u64 != c.dims {
                        return err(400, "Wrong input: Vector dimension error");
                    }
                    let Value::Object(payload) = p["payload"].clone() else {
                        return err(400, "payload must be an object");
                    };
                    c.points.insert(id, (vector, payload));
                }
                ok(json!({"status": "completed"}))
            }
            ("POST", ["collections", name, "points", op]) => {
                let Some(c) = s.collections.get_mut(*name) else {
                    return err(404, "Not found");
                };
                let filter = body.get("filter").cloned().unwrap_or(json!({}));
                match *op {
                    "query" => {
                        let q: Vec<f32> = body["query"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|x| x.as_f64().unwrap() as f32)
                            .collect();
                        let limit = body["limit"].as_u64().unwrap_or(10) as usize;
                        let threshold = body.get("score_threshold").and_then(Value::as_f64);
                        let ignore = self.ignore_search_filter.load(Ordering::SeqCst);
                        let mut hits: Vec<(String, f64, Map<String, Value>)> = c
                            .points
                            .iter()
                            .filter(|(id, (_, p))| ignore || filter_matches(&filter, id, p))
                            .map(|(id, (v, p))| (id.clone(), cosine(&q, v), p.clone()))
                            .filter(|(_, s, _)| threshold.is_none_or(|t| *s >= t))
                            .collect();
                        hits.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                        hits.truncate(limit);
                        ok(
                            json!({"points": hits.into_iter().map(|(id, score, payload)| json!({
                            "id": id, "version": 0, "score": score, "payload": payload
                        })).collect::<Vec<_>>()}),
                        )
                    }
                    "scroll" => {
                        let limit = body["limit"].as_u64().unwrap_or(10) as usize;
                        let offset = body.get("offset").and_then(Value::as_str);
                        let with_vector = body["with_vector"].as_bool().unwrap_or(false);
                        let with_payload = body.get("with_payload").cloned().unwrap_or(json!(true));
                        let matching: Vec<_> = c
                            .points
                            .iter()
                            .filter(|(id, (_, p))| filter_matches(&filter, id, p))
                            .filter(|(id, _)| offset.is_none_or(|o| id.as_str() >= o))
                            .collect();
                        let page: Vec<Value> = matching
                            .iter()
                            .take(limit)
                            .map(|(id, (v, p))| {
                                let mut item =
                                    json!({"id": id, "payload": select_payload(p, &with_payload)});
                                if with_vector {
                                    item["vector"] = json!(v);
                                }
                                item
                            })
                            .collect();
                        let next = matching
                            .get(limit)
                            .map(|(id, _)| json!(id))
                            .unwrap_or(Value::Null);
                        ok(json!({"points": page, "next_page_offset": next}))
                    }
                    "count" => {
                        let n = c
                            .points
                            .iter()
                            .filter(|(id, (_, p))| filter_matches(&filter, id, p))
                            .count();
                        ok(json!({"count": n}))
                    }
                    "delete" => {
                        let ids: Vec<String> = c
                            .points
                            .iter()
                            .filter(|(id, (_, p))| filter_matches(&filter, id, p))
                            .map(|(id, _)| id.clone())
                            .collect();
                        for id in ids {
                            c.points.remove(&id);
                        }
                        ok(json!({"status": "completed"}))
                    }
                    "payload" => {
                        let Value::Object(update) = body["payload"].clone() else {
                            return err(400, "payload must be an object");
                        };
                        for (id, (_, p)) in c.points.iter_mut() {
                            if filter_matches(&filter, id, p) {
                                for (k, v) in &update {
                                    p.insert(k.clone(), v.clone());
                                }
                            }
                        }
                        ok(json!({"status": "completed"}))
                    }
                    _ => err(404, "unknown points op"),
                }
            }
            _ => err(404, "unknown endpoint"),
        }
    }
}

impl Respond for FakeQdrant {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let method = request.method.as_str().to_owned();
        let path = request.url.path().to_owned();
        self.state
            .lock()
            .unwrap()
            .log
            .push((method.clone(), path.clone(), body.clone()));
        self.handle(&method, &path, &body)
    }
}
