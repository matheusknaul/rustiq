//! Immutable payload pairs, encrypted with their owning request. No cookie store
//! or full RequestOptions/HttpResponse is serialized into a history record.
use crate::workspace::{Id, Workspace};
use rustiq_http::{HttpResponse, PayloadSnapshot};
use serde::{Deserialize, Serialize};

pub const MAX_RECORDS_PER_REQUEST: usize = 100;
pub const MAX_RECORD_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_HISTORY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exchange {
    pub id: String,
    pub sent_at: u64,
    pub status: u16,
    pub elapsed_ms: u128,
    pub response_size: usize,
    pub request: PayloadSnapshot,
    pub response: PayloadSnapshot,
}
impl std::fmt::Debug for Exchange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Exchange")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}
impl Exchange {
    pub fn from_response(response: &HttpResponse) -> Option<Self> {
        let mut request = response.request_payload.clone()?;
        request.exclude_cookies();
        Some(Self {
            id: uuid::Uuid::new_v4().to_string(),
            sent_at: response.sent_at,
            status: response.status,
            elapsed_ms: response.elapsed_ms,
            response_size: response.size,
            request,
            response: PayloadSnapshot::new(response.body.clone(), response.headers.clone()),
        })
    }
    pub fn byte_len(&self) -> usize {
        self.request
            .byte_len()
            .saturating_add(self.response.byte_len())
            .saturating_add(128)
    }
    pub fn sanitize(&mut self) {
        self.request.exclude_cookies();
        self.response.exclude_cookies();
    }
}

pub fn record(
    workspace: &mut Workspace,
    request_id: Id,
    response: &HttpResponse,
) -> Result<bool, &'static str> {
    let Some(exchange) = Exchange::from_response(response) else {
        return Ok(false);
    };
    if exchange.byte_len() > MAX_RECORD_BYTES {
        return Err("Response received. History skipped: request/response payloads exceed 2 MiB.");
    }
    let Some(entry) = workspace.entry_mut(request_id) else {
        return Ok(false);
    };
    if entry.kind != crate::workspace::Kind::Request {
        return Ok(false);
    }
    entry.history.push(exchange);
    enforce_limits(workspace);
    Ok(true)
}

pub fn enforce_limits(workspace: &mut Workspace) {
    for entry in &mut workspace.entries {
        entry
            .history
            .retain(|exchange| exchange.byte_len() <= MAX_RECORD_BYTES);
        let excess = entry.history.len().saturating_sub(MAX_RECORDS_PER_REQUEST);
        entry.history.drain(..excess);
    }
    let mut total: usize = workspace
        .entries
        .iter()
        .flat_map(|e| &e.history)
        .map(Exchange::byte_len)
        .sum();
    while total > MAX_HISTORY_BYTES {
        // Timestamps can tie, so use entry/record indices as deterministic tie breakers.
        let oldest = workspace
            .entries
            .iter()
            .enumerate()
            .flat_map(|(entry, e)| {
                e.history
                    .iter()
                    .enumerate()
                    .map(move |(record, h)| (h.sent_at, entry, record))
            })
            .min();
        let Some((_, entry, record)) = oldest else {
            break;
        };
        total = total.saturating_sub(workspace.entries[entry].history.remove(record).byte_len());
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::workspace::Kind;
    pub fn response() -> HttpResponse {
        HttpResponse {
            status: 201,
            sent_at: 123,
            body: "response-secret".into(),
            size: 15,
            request_payload: Some(PayloadSnapshot {
                body: "original-request".into(),
                headers: vec![
                    ("Authorization".into(), "Bearer secret".into()),
                    ("Cookie".into(), "session=excluded".into()),
                ],
            }),
            headers: vec![
                ("X-Result".into(), "response-header".into()),
                ("SET-cookie".into(), "session=excluded".into()),
            ],
            ..Default::default()
        }
    }
    fn workspace() -> (Workspace, Id) {
        let mut workspace = Workspace::default();
        let collection = workspace.create(Kind::Collection, None, "API").unwrap();
        let id = workspace
            .create(Kind::Request, Some(collection), "Request")
            .unwrap();
        (workspace, id)
    }
    #[test]
    fn only_payload_pairs_are_kept_without_any_cookie_headers() {
        let (mut workspace, id) = workspace();
        record(&mut workspace, id, &response()).unwrap();
        let record = &workspace.entry(id).unwrap().history[0];
        assert_eq!(record.request.body, "original-request");
        assert_eq!(record.response.body, "response-secret");
        assert_eq!(record.request.headers.len(), 1);
        assert_eq!(record.response.headers.len(), 1);
        let json = serde_json::to_string(record).unwrap();
        assert!(!json.contains("excluded"));
        assert!(!json.contains("cookies"));
        assert!(!json.contains("url"));
        assert!(!format!("{record:?}").contains("secret"));
    }
    #[test]
    fn retention_evicts_old_entries_and_rejects_oversized_payloads() {
        let (mut workspace, id) = workspace();
        for i in 0..MAX_RECORDS_PER_REQUEST + 2 {
            let mut response = response();
            response.sent_at = i as u64;
            record(&mut workspace, id, &response).unwrap();
        }
        assert_eq!(
            workspace.entry(id).unwrap().history.len(),
            MAX_RECORDS_PER_REQUEST
        );
        assert_eq!(workspace.entry(id).unwrap().history[0].sent_at, 2);
        let mut large = response();
        large.body = "x".repeat(MAX_RECORD_BYTES);
        assert!(record(&mut workspace, id, &large).is_err());
        assert_eq!(
            workspace.entry(id).unwrap().history.len(),
            MAX_RECORDS_PER_REQUEST
        );
    }
    #[test]
    fn global_byte_limit_is_applied_across_requests() {
        let (mut workspace, id) = workspace();
        let second = workspace.create(Kind::Request, Some(1), "Second").unwrap();
        for i in 0..12 {
            let mut response = response();
            response.sent_at = i;
            response.body = "x".repeat(1024 * 1024);
            record(
                &mut workspace,
                if i % 2 == 0 { id } else { second },
                &response,
            )
            .unwrap();
        }
        let all: Vec<_> = workspace.entries.iter().flat_map(|e| &e.history).collect();
        assert!(all.iter().map(|h| h.byte_len()).sum::<usize>() <= MAX_HISTORY_BYTES);
        assert!(all.iter().all(|h| h.sent_at >= 5));
    }
}
