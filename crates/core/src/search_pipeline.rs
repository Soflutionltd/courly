//! Unified search pipeline that routes queries through SQL, Tantivy, or both.
//!
//! Entry point: [`search()`] parses the query string and dispatches to the
//! appropriate backend based on whether the query contains free text,
//! structured operators, or both.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::db::ReadConn;
use db_read::db::FromRow;
use db_read::db::sql_fragments::LATEST_MESSAGE_SUBQUERY;
use db_read::db::types::{AccountScope, DbThread};
use search::{
    AttachmentAttributionInput, AttributionInputs, MatchKind, SearchParams, SearchReadState,
    SearchResult as TantivyResult,
};
use smart_folder::{ParsedQuery, parse_query, query_thread_keys_read, query_threads_read};

// ── Result type ─────────────────────────────────────────────

/// A unified search result that works for all three search paths.
#[derive(Debug, Clone)]
pub struct UnifiedSearchResult {
    pub thread_id: String,
    pub account_id: String,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub date: Option<i64>,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
    pub message_count: Option<i64>,
    /// BM25 score from Tantivy, or 0.0 for SQL-only results.
    pub rank: f32,
    /// Phase 7-8: which field carried the primary match.
    /// `None` means this result came from a path where attribution is
    /// unavailable or meaningless, such as operator-only SQL.
    pub match_kind: Option<MatchKind>,
    /// Phase 7-8: secondary matches above the 50%-of-top-score
    /// threshold, score-descending. Empty for SQL paths.
    pub also_matched: Vec<MatchKind>,
}

/// Search result quality is a property of the result set, not each row.
#[derive(Debug, Clone)]
pub enum SearchResults {
    FullIndex(Vec<UnifiedSearchResult>),
    Degraded(Vec<UnifiedSearchResult>),
}

// ── Public entry point ──────────────────────────────────────

/// Parse a query string and route it through the appropriate search backend(s).
///
/// - Empty query returns no results.
/// - Operators only (e.g. `is:unread from:alice`) routes through SQL.
/// - Free text only (e.g. `meeting notes`) routes through Tantivy.
/// - Both operators and free text intersects SQL candidates with Tantivy scores.
///
/// `scope` and `folder_id` narrow every path before ranking or limiting,
/// so a scoped search never loses matches to other accounts or folders.
/// An `account:` operator in the query overrides `scope`.
pub fn search(
    query: &str,
    search_state: Option<&SearchReadState>,
    conn: &ReadConn<'_>,
    scope: &AccountScope,
    folder_id: Option<&str>,
    body_read: Option<&store::body_store::BodyStoreReadState>,
) -> Result<SearchResults, String> {
    let mut parsed = parse_query(query);
    let has_query = !parsed.free_text.is_empty() || parsed.has_any_operator();
    parsed.scope_folder_id = folder_id.map(str::to_owned);

    let has_free_text = !parsed.free_text.is_empty();
    let has_operators = parsed.has_any_operator();

    if !has_query {
        return Ok(SearchResults::FullIndex(Vec::new()));
    }

    let Some(search_state) = search_state else {
        let results = search_sql_fallback(&parsed, conn, scope)?;
        log::info!(
            "Search executed via degraded_sql_fallback path, returned {} results",
            results.len()
        );
        return Ok(SearchResults::Degraded(results));
    };

    let path_name = match (has_free_text, has_operators) {
        (false, false) => unreachable!("empty search returned before routing"),
        (false, true) => "sql_only",
        (true, false) => "tantivy_only",
        (true, true) => "combined",
    };
    log::debug!("Search pipeline routing: path={path_name}, query={query:?}");

    let started = Instant::now();
    let result = match (has_free_text, has_operators) {
        (false, false) => unreachable!("empty search returned before routing"),
        (false, true) => search_sql_only(&parsed, conn, scope),
        (true, false) => search_tantivy_only(&parsed, search_state, conn, scope, body_read),
        (true, true) => search_combined(&parsed, search_state, conn, scope, body_read),
    };

    match &result {
        Ok(results) => {
            log::info!(
                "Search executed via {path_name} path, returned {} results in {} ms",
                results.len(),
                started.elapsed().as_millis()
            );
        }
        Err(e) => {
            log::error!("Search failed via {path_name} path: {e}");
        }
    }

    result.map(SearchResults::FullIndex)
}

/// SQL-only fallback for cases where no Tantivy search state is available.
/// Kept private so callers route through `search()` and handle degraded
/// quality explicitly.
fn search_sql_fallback(
    parsed: &ParsedQuery,
    conn: &ReadConn<'_>,
    scope: &AccountScope,
) -> Result<Vec<UnifiedSearchResult>, String> {
    let scope = scope.clone();

    if parsed.has_any_operator() || parsed.free_text.is_empty() {
        let db_threads = query_threads_read(conn, parsed, &scope, Some(200), Some(0))?;
        Ok(db_threads.into_iter().map(db_thread_to_unified).collect())
    } else {
        let pattern = format!("%{}%", parsed.free_text);
        let rows = crate::db::queries_extra::search_fallback::search_threads_freetext_sync(
            conn, &pattern, &scope, 200,
        )?;
        Ok(rows
            .into_iter()
            .map(|r| UnifiedSearchResult {
                thread_id: r.thread_id,
                account_id: r.account_id,
                subject: r.subject,
                snippet: r.snippet,
                from_name: r.from_name,
                from_address: r.from_address,
                date: r.last_message_at,
                is_read: r.is_read,
                is_starred: r.is_starred,
                has_attachments: r.has_attachments,
                message_count: Some(r.message_count),
                rank: 0.0,
                match_kind: None,
                also_matched: Vec::new(),
            })
            .collect())
    }
}

// ── Path 1: SQL only ────────────────────────────────────────

/// Operators without free text: run SQL query, return date-sorted results.
fn search_sql_only(
    parsed: &ParsedQuery,
    conn: &ReadConn<'_>,
    scope: &AccountScope,
) -> Result<Vec<UnifiedSearchResult>, String> {
    let threads = query_threads_read(conn, parsed, scope, Some(200), Some(0))?;
    Ok(threads.into_iter().map(db_thread_to_unified).collect())
}

// ── Path 2: Tantivy only ────────────────────────────────────

/// Free text without operators: run Tantivy search, group by thread.
fn search_tantivy_only(
    parsed: &ParsedQuery,
    search_state: &SearchReadState,
    conn: &ReadConn<'_>,
    scope: &AccountScope,
    body_read: Option<&store::body_store::BodyStoreReadState>,
) -> Result<Vec<UnifiedSearchResult>, String> {
    let mut params = build_tantivy_params(parsed);
    params.account_ids = scope_account_ids(scope);
    let index_started = Instant::now();
    let mut results = search_state.search_with_filters(&params)?;
    let index_ms = index_started.elapsed().as_millis();
    let attribution_started = Instant::now();
    let excerpts = enrich_with_attribution(
        &mut results,
        &parsed.free_text,
        search_state,
        conn,
        body_read,
    );
    let attribution_ms = attribution_started.elapsed().as_millis();
    let hits = results.len();
    let mut grouped = group_by_thread_unified(results);
    let hydrate_started = Instant::now();
    let thread_rows = fetch_thread_rows_for_results(conn, &grouped)?;
    log::info!(
        "Search timing tantivy_only: index {index_ms} ms for {hits} hits, attribution {attribution_ms} ms, thread rows {} ms",
        hydrate_started.elapsed().as_millis()
    );
    let thread_map: HashMap<(String, String), &DbThread> = thread_rows
        .iter()
        .map(|thread| ((thread.account_id.clone(), thread.id.clone()), thread))
        .collect();
    grouped = grouped
        .into_iter()
        .filter_map(|r| {
            let key = (r.account_id.clone(), r.thread_id.clone());
            thread_map
                .get(&key)
                .map(|thread| enrich_from_sql(r, thread))
        })
        .collect();
    preview_matches(&mut grouped, &excerpts, &parsed.free_text);
    grouped.sort_by(newest_first);
    Ok(grouped)
}

// ── Path 3: Combined ────────────────────────────────────────

/// Both operators and free text: SQL narrows candidates, Tantivy scores them.
fn search_combined(
    parsed: &ParsedQuery,
    search_state: &SearchReadState,
    conn: &ReadConn<'_>,
    scope: &AccountScope,
    body_read: Option<&store::body_store::BodyStoreReadState>,
) -> Result<Vec<UnifiedSearchResult>, String> {
    // Step 1: SQL narrows the corpus by structured operators ONLY, returning
    // an exhaustive list of (account_id, thread_id) candidate keys (no LIMIT,
    // no hydration). Free-text matching belongs to Tantivy on the combined
    // path - SQL's `build_free_text_clause` does a subject/snippet/from LIKE
    // without tokenisation, so if it ran here it would shrink the candidate
    // set below what Tantivy can usefully rank. The query is cloned with
    // `free_text` cleared so the SQL builder treats this as operators-only.
    //
    // Exhaustive (no LIMIT) is required: capping at DEFAULT_QUERY_LIMIT would
    // truncate older threads before Tantivy gets a chance to rank them, so
    // the actual best free-text match could be silently excluded.
    let mut sql_parsed = parsed.clone();
    sql_parsed.free_text.clear();
    let candidates_started = Instant::now();
    let candidate_pairs = query_thread_keys_read(conn, &sql_parsed, scope)?;
    let candidates_ms = candidates_started.elapsed().as_millis();
    let candidates = candidate_pairs.len();
    if candidate_pairs.is_empty() {
        return Ok(Vec::new());
    }

    // Step 2: Tantivy ranks free-text matches WITHIN the SQL candidate set.
    // Operator filters are not re-applied here - SQL is the sole constraint
    // authority on the combined path (Tantivy's text-match semantics on
    // those fields don't always agree with SQL's contact-expanded ones).
    // `thread_filter` pins the response to the candidate set inside the
    // engine so the result is a true narrow-then-rank rather than
    // broad-then-intersect.
    let mut params = build_tantivy_params(parsed);
    params.account_ids = None;
    params.from = Vec::new();
    params.to = Vec::new();
    params.has_attachment = None;
    params.is_unread = None;
    params.is_starred = None;
    params.before = None;
    params.after = None;
    params.thread_filter = Some(candidate_pairs);
    let index_started = Instant::now();
    let mut tantivy_results = search_state.search_with_filters(&params)?;
    let index_ms = index_started.elapsed().as_millis();

    // Phase 7-8: per-message match-kind attribution. Runs before
    // grouping so the highest-scoring message's attribution survives
    // thread-grouping intact.
    let attribution_started = Instant::now();
    let excerpts = enrich_with_attribution(
        &mut tantivy_results,
        &parsed.free_text,
        search_state,
        conn,
        body_read,
    );
    let attribution_ms = attribution_started.elapsed().as_millis();
    let hits = tantivy_results.len();

    // Step 3: Group by thread, take max score.
    let grouped = group_by_thread_unified(tantivy_results);

    // Step 4: Hydrate latest-message metadata for the top-N hits only.
    // fetch_thread_rows_for_results does a batched VALUES-list JOIN so this
    // is one round trip regardless of N.
    let hydrate_started = Instant::now();
    let hydrated = fetch_thread_rows_for_results(conn, &grouped)?;
    log::info!(
        "Search timing combined: sql candidates {candidates_ms} ms for {candidates} threads, index {index_ms} ms for {hits} hits, attribution {attribution_ms} ms, thread rows {} ms",
        hydrate_started.elapsed().as_millis()
    );
    let thread_map: HashMap<(String, String), &DbThread> = hydrated
        .iter()
        .map(|t| ((t.account_id.clone(), t.id.clone()), t))
        .collect();

    // Step 5: Re-project each Tantivy hit onto the thread's latest-message
    // metadata. filter_map is defensive: thread_filter guarantees every hit
    // is in the SQL candidate set, but a thread could in theory be deleted
    // between the SQL candidate fetch and the hydration step.
    let mut enriched: Vec<UnifiedSearchResult> = grouped
        .into_iter()
        .filter_map(|r| {
            let key = (r.account_id.clone(), r.thread_id.clone());
            thread_map
                .get(&key)
                .map(|thread| enrich_from_sql(r, thread))
        })
        .collect();

    preview_matches(&mut enriched, &excerpts, &parsed.free_text);

    // Step 6: Newest conversation first.
    enriched.sort_by(newest_first);
    Ok(enriched)
}

/// Search results read like a mailbox: the conversation with the most
/// recent mail first, relevance only breaking ties. A conversation without
/// a date goes last.
fn newest_first(a: &UnifiedSearchResult, b: &UnifiedSearchResult) -> std::cmp::Ordering {
    b.date.cmp(&a.date).then_with(|| {
        b.rank
            .partial_cmp(&a.rank)
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

// ── Helpers ─────────────────────────────────────────────────

/// Phase 7-8: collect per-message attribution inputs from canonical DB
/// state and rewrite each result's `match_kind` / `also_matched` to
/// reflect which field actually matched.
///
/// M2 fix: body_text is now fetched from `body_store` via the optional
/// `body_read` parameter. When `None` (caller didn't have a
/// BodyStoreReadState handy), body falls through to empty as before
/// and the attribution can only score subject / from / attachments;
/// when `Some`, a sync batched read populates body_text per message
/// and the attribution can correctly attribute body matches.
///
/// On any DB error we log a warning and leave the results' default
/// `MatchKind::Body` in place; this is a UI-affordance feature and
/// must never break a search.
///
/// Returns, per conversation, the line of body text around the searched
/// word in its most recent message that holds it: the bodies are read here
/// anyway, and `preview_matches` shows that line where a row would
/// otherwise show no trace of the word.
fn enrich_with_attribution(
    results: &mut [TantivyResult],
    free_text: &str,
    search_state: &SearchReadState,
    conn: &ReadConn<'_>,
    body_read: Option<&store::body_store::BodyStoreReadState>,
) -> MatchExcerpts {
    let mut excerpts = MatchExcerpts::new();
    if free_text.trim().is_empty() || results.is_empty() {
        return excerpts;
    }
    let pairs: Vec<(String, String)> = results
        .iter()
        .map(|r| (r.account_id.clone(), r.message_id.clone()))
        .collect();
    let fragments_started = Instant::now();
    let fragments =
        match db_read::db::queries_extra::select_attachment_fragments_batch(conn, &pairs) {
            Ok(map) => map,
            Err(e) => {
                log::warn!("enrich_with_attribution: attachment fetch failed: {e}");
                return excerpts;
            }
        };
    let fragments_ms = fragments_started.elapsed().as_millis();
    let attachment_bytes: usize = fragments
        .values()
        .flatten()
        .map(|row| row.extracted_text.len())
        .sum();
    // Body fetch is opt-in via `body_read`; failures fall back to empty
    // body strings (degraded but not broken).
    let bodies_started = Instant::now();
    let mut body_by_mid: HashMap<String, String> = HashMap::new();
    if let Some(body_read) = body_read {
        let message_ids: Vec<String> = results.iter().map(|r| r.message_id.clone()).collect();
        match body_read.get_batch_sync(&message_ids) {
            Ok(bodies) => {
                for b in bodies {
                    if let Some(text) = b.body_text {
                        body_by_mid.insert(b.message_id, text);
                    }
                }
            }
            Err(e) => {
                log::warn!("enrich_with_attribution: body fetch failed: {e}");
            }
        }
    }
    let bodies_ms = bodies_started.elapsed().as_millis();
    let body_bytes: usize = body_by_mid.values().map(String::len).sum();
    let scoring_started = Instant::now();
    // Results come newest first, so the first excerpt of a conversation is
    // the one of its most recent matching message.
    let words = crate::search_match::free_text_words(free_text);
    for r in results.iter() {
        let key = (r.account_id.clone(), r.thread_id.clone());
        if excerpts.contains_key(&key) {
            continue;
        }
        let line = body_by_mid
            .get(&r.message_id)
            .and_then(|text| crate::search_match::excerpt(text, &words));
        if let Some(line) = line {
            excerpts.insert(key, line);
        }
    }
    let mut inputs: HashMap<String, AttributionInputs> = HashMap::with_capacity(results.len());
    for r in results.iter() {
        let key = (r.account_id.clone(), r.message_id.clone());
        let attachments: Vec<AttachmentAttributionInput> = fragments
            .get(&key)
            .map(|rows| {
                rows.iter()
                    .map(|row| AttachmentAttributionInput {
                        attachment_id: row.attachment_id.clone(),
                        filename: row.filename.clone(),
                        mime: row.mime_type.clone(),
                        extracted_text: row.extracted_text.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        inputs.insert(
            r.message_id.clone(),
            AttributionInputs {
                subject: r.subject.clone().unwrap_or_default(),
                from_name: r.from_name.clone().unwrap_or_default(),
                body_text: body_by_mid.remove(&r.message_id).unwrap_or_default(),
                attachments,
            },
        );
    }
    if let Err(e) = search_state.enrich_match_kinds(free_text, results, &inputs) {
        log::warn!("enrich_with_attribution: enrich_match_kinds failed: {e}");
    }
    log::info!(
        "Search timing attribution: attachment text {fragments_ms} ms for {attachment_bytes} bytes, bodies {bodies_ms} ms for {body_bytes} bytes, scoring {} ms",
        scoring_started.elapsed().as_millis()
    );
    excerpts
}

/// The line of body text around the searched word, per conversation
/// `(account_id, thread_id)`.
type MatchExcerpts = HashMap<(String, String), String>;

/// Preview a result with the line of its body that holds the searched word,
/// when nothing the row shows does: its sender, its subject and the usual
/// preview all come from the latest message of the conversation, and the
/// word may sit further down or in an earlier message. A row that already
/// shows the word keeps its preview.
fn preview_matches(results: &mut [UnifiedSearchResult], excerpts: &MatchExcerpts, free_text: &str) {
    let words = crate::search_match::free_text_words(free_text);
    for result in results {
        let shown = [
            &result.from_name,
            &result.from_address,
            &result.subject,
            &result.snippet,
        ];
        if shown
            .into_iter()
            .flatten()
            .any(|text| crate::search_match::holds_match(text, &words))
        {
            continue;
        }
        let key = (result.account_id.clone(), result.thread_id.clone());
        if let Some(line) = excerpts.get(&key) {
            result.snippet = Some(line.clone());
        }
    }
}

/// Determine the account scope from parsed query operators.
///
/// The account ids a Tantivy search is limited to, `None` for every account.
fn scope_account_ids(scope: &AccountScope) -> Option<Vec<String>> {
    match scope {
        AccountScope::All => None,
        AccountScope::Single(id) => Some(vec![id.clone()]),
        AccountScope::Multiple(ids) => Some(ids.clone()),
    }
}

/// Build Tantivy `SearchParams` from a parsed query.
fn build_tantivy_params(parsed: &ParsedQuery) -> SearchParams {
    let account_ids = if parsed.account.is_empty() {
        None
    } else {
        Some(parsed.account.clone())
    };

    SearchParams {
        account_ids,
        free_text: Some(parsed.free_text.clone()),
        from: parsed.from.clone(),
        to: parsed.to.clone(),
        subject: None,
        has_attachment: if parsed.has_attachment {
            Some(true)
        } else {
            None
        },
        is_unread: parsed.is_unread,
        is_starred: parsed.is_starred,
        before: parsed.before,
        after: parsed.after,
        thread_filter: None,
        limit: Some(200),
    }
}

/// Convert a `DbThread` (from SQL) into a `UnifiedSearchResult` with rank 0.0.
fn db_thread_to_unified(t: DbThread) -> UnifiedSearchResult {
    UnifiedSearchResult {
        thread_id: t.id,
        account_id: t.account_id,
        subject: t.subject,
        snippet: t.snippet,
        from_name: t.from_name,
        from_address: t.from_address,
        date: t.last_message_at,
        is_read: t.is_read,
        is_starred: t.is_starred,
        has_attachments: t.has_attachments,
        message_count: Some(t.message_count),
        rank: 0.0,
        match_kind: None,
        also_matched: Vec::new(),
    }
}

/// Group message-level Tantivy results by thread_id, taking the highest
/// score per thread. Delegates to `search::group_by_thread`
/// for the grouping logic, then converts to `UnifiedSearchResult`.
fn group_by_thread_unified(results: Vec<TantivyResult>) -> Vec<UnifiedSearchResult> {
    let grouped = search::group_by_thread(results);
    grouped
        .into_iter()
        .map(|r| tantivy_result_to_unified(&r))
        .collect()
}

fn fetch_thread_rows_for_results(
    conn: &ReadConn<'_>,
    results: &[UnifiedSearchResult],
) -> Result<Vec<DbThread>, String> {
    let mut seen = HashSet::new();
    let mut keys = Vec::new();
    for result in results {
        if !seen.insert((result.account_id.clone(), result.thread_id.clone())) {
            continue;
        }
        keys.push((result.account_id.clone(), result.thread_id.clone()));
    }
    if keys.is_empty() {
        return Ok(Vec::new());
    }

    let values_sql = keys
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let account_idx = index * 2 + 1;
            let thread_idx = account_idx + 1;
            format!("(?{account_idx}, ?{thread_idx})")
        })
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "WITH requested(account_id, thread_id) AS (VALUES {values_sql})
         SELECT t.*, m.from_name, m.from_address
         FROM requested r
         INNER JOIN threads t ON t.account_id = r.account_id AND t.id = r.thread_id
         LEFT JOIN ({LATEST_MESSAGE_SUBQUERY}
         ) m ON m.account_id = t.account_id AND m.thread_id = t.id"
    );

    let mut params: Vec<Box<dyn db_read::db::ToSql>> = Vec::with_capacity(keys.len() * 2);
    for (account_id, thread_id) in keys {
        params.push(Box::new(account_id));
        params.push(Box::new(thread_id));
    }
    let param_refs: Vec<&dyn db_read::db::ToSql> = params.iter().map(AsRef::as_ref).collect();
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("prepare search thread metadata: {e}"))?;
    stmt.query_map(param_refs.as_slice(), DbThread::from_row)
        .map_err(|e| format!("query search thread metadata: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("map search thread metadata: {e}"))
}

/// Convert a single Tantivy result into a `UnifiedSearchResult`.
/// Propagates the `match_kind` + `also_matched` fields populated by
/// `enrich_with_attribution` (phase 7-8).
fn tantivy_result_to_unified(r: &TantivyResult) -> UnifiedSearchResult {
    UnifiedSearchResult {
        thread_id: r.thread_id.clone(),
        account_id: r.account_id.clone(),
        subject: r.subject.clone(),
        snippet: r.snippet.clone(),
        from_name: r.from_name.clone(),
        from_address: r.from_address.clone(),
        date: Some(r.date),
        is_read: false,
        is_starred: false,
        has_attachments: false,
        message_count: None,
        rank: r.rank,
        match_kind: Some(r.match_kind.clone()),
        also_matched: r.also_matched.clone(),
    }
}

/// Override display fields with the thread's latest-message metadata per the
/// product spec; keep Tantivy's `rank` / `match_kind` / `also_matched` so the
/// hit explains why this thread matched while the card looks the same as in
/// the inbox.
fn enrich_from_sql(mut result: UnifiedSearchResult, thread: &DbThread) -> UnifiedSearchResult {
    result.subject = thread.subject.clone();
    result.snippet = thread.snippet.clone();
    result.from_name = thread.from_name.clone();
    result.from_address = thread.from_address.clone();
    result.date = thread.last_message_at;
    result.is_read = thread.is_read;
    result.is_starred = thread.is_starred;
    result.has_attachments = thread.has_attachments;
    result.message_count = Some(thread.message_count);
    result
}

/// Parse a date string (ISO 8601 or unix timestamp) into epoch seconds.
#[allow(dead_code)] // helper kept for upcoming smart-folder date pipeline
fn parse_date_string(s: &str) -> Option<i64> {
    s.parse::<i64>().ok()
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    // -- Routing logic --

    #[test]
    fn empty_query_returns_empty() {
        let parsed = parse_query("");
        assert!(parsed.free_text.is_empty());
        assert!(!parsed.has_any_operator());
    }

    #[test]
    fn free_text_only_detected() {
        let parsed = parse_query("meeting notes");
        assert!(!parsed.free_text.is_empty());
        assert!(!parsed.has_any_operator());
    }

    #[test]
    fn operators_only_detected() {
        let parsed = parse_query("is:unread from:alice");
        assert!(parsed.free_text.is_empty());
        assert!(parsed.has_any_operator());
    }

    #[test]
    fn combined_detected() {
        let parsed = parse_query("meeting from:alice");
        assert!(!parsed.free_text.is_empty());
        assert!(parsed.has_any_operator());
    }

    // -- Scope building --

    #[test]
    fn every_account_leaves_tantivy_unfiltered() {
        assert!(scope_account_ids(&AccountScope::All).is_none());
    }

    #[test]
    fn a_scoped_account_filters_tantivy() {
        assert_eq!(
            scope_account_ids(&AccountScope::Single("acc1".to_owned())),
            Some(vec!["acc1".to_owned()])
        );
        assert_eq!(
            scope_account_ids(&AccountScope::Multiple(vec![
                "acc1".to_owned(),
                "acc2".to_owned()
            ])),
            Some(vec!["acc1".to_owned(), "acc2".to_owned()])
        );
    }

    // -- Tantivy params building --

    #[test]
    fn tantivy_params_basic() {
        let parsed = parse_query("hello world");
        let params = build_tantivy_params(&parsed);
        assert_eq!(params.free_text.as_deref(), Some("hello world"));
        assert!(params.account_ids.is_none());
        assert!(params.from.is_empty());
    }

    #[test]
    fn tantivy_params_with_account() {
        let parsed = parse_query("hello account:work");
        let params = build_tantivy_params(&parsed);
        assert_eq!(params.free_text.as_deref(), Some("hello"));
        assert_eq!(params.account_ids, Some(vec!["work".to_owned()]));
    }

    #[test]
    fn tantivy_params_with_filters() {
        let parsed = parse_query("hello from:alice has:attachment is:unread");
        let params = build_tantivy_params(&parsed);
        assert_eq!(params.free_text.as_deref(), Some("hello"));
        assert_eq!(params.from.first().map(String::as_str), Some("alice"));
        assert_eq!(params.has_attachment, Some(true));
        assert_eq!(params.is_unread, Some(true));
    }

    // -- DbThread to UnifiedSearchResult conversion --

    #[test]
    fn db_thread_converts_correctly() {
        let thread = DbThread {
            id: "t1".to_owned(),
            account_id: "acc1".to_owned(),
            subject: Some("Test subject".to_owned()),
            snippet: Some("Test snippet".to_owned()),
            last_message_at: Some(1_700_000_000),
            message_count: 3,
            is_read: true,
            is_starred: false,
            is_important: false,
            has_attachments: true,
            is_snoozed: false,
            snooze_until: None,
            is_pinned: false,
            is_muted: false,
            is_flag_complete: false,
            from_name: Some("Alice".to_owned()),
            from_address: Some("alice@test.com".to_owned()),
            to_addresses: None,
        };

        let result = db_thread_to_unified(thread);
        assert_eq!(result.thread_id, "t1");
        assert_eq!(result.account_id, "acc1");
        assert_eq!(result.subject.as_deref(), Some("Test subject"));
        assert_eq!(result.date, Some(1_700_000_000));
        assert!(result.is_read);
        assert!(!result.is_starred);
        assert_eq!(result.message_count, Some(3));
        assert_eq!(result.rank, 0.0);
        assert!(result.match_kind.is_none());
    }

    #[test]
    fn results_are_newest_first_with_relevance_breaking_ties() {
        let result = |thread_id: &str, date: Option<i64>, rank: f32| UnifiedSearchResult {
            thread_id: thread_id.into(),
            account_id: "acct".into(),
            subject: None,
            snippet: None,
            from_name: None,
            from_address: None,
            date,
            is_read: true,
            is_starred: false,
            has_attachments: false,
            message_count: None,
            rank,
            match_kind: None,
            also_matched: Vec::new(),
        };
        let mut results = [
            result("old-strong", Some(100), 9.0),
            result("undated", None, 9.0),
            result("new-weak", Some(300), 1.0),
            result("mid-weak", Some(200), 1.0),
            result("mid-strong", Some(200), 5.0),
        ];

        results.sort_by(newest_first);

        let order: Vec<&str> = results.iter().map(|r| r.thread_id.as_str()).collect();
        assert_eq!(
            order,
            [
                "new-weak",
                "mid-strong",
                "mid-weak",
                "old-strong",
                "undated"
            ]
        );
    }

    // -- group_by_thread --

    #[test]
    fn group_by_thread_takes_max_score() {
        let results = vec![
            TantivyResult {
                message_id: "m1".to_owned(),
                account_id: "acc1".to_owned(),
                thread_id: "t1".to_owned(),
                subject: Some("A".to_owned()),
                from_name: None,
                from_address: None,
                snippet: None,
                date: 1000,
                rank: 2.5,
                match_kind: search::MatchKind::Body,
                also_matched: Vec::new(),
            },
            TantivyResult {
                message_id: "m2".to_owned(),
                account_id: "acc1".to_owned(),
                thread_id: "t1".to_owned(),
                subject: Some("B".to_owned()),
                from_name: None,
                from_address: None,
                snippet: None,
                date: 2000,
                rank: 5.0,
                match_kind: search::MatchKind::Body,
                also_matched: Vec::new(),
            },
            TantivyResult {
                message_id: "m3".to_owned(),
                account_id: "acc2".to_owned(),
                thread_id: "t2".to_owned(),
                subject: Some("C".to_owned()),
                from_name: None,
                from_address: None,
                snippet: None,
                date: 3000,
                rank: 1.0,
                match_kind: search::MatchKind::Body,
                also_matched: Vec::new(),
            },
        ];

        let grouped = group_by_thread_unified(results);
        assert_eq!(grouped.len(), 2);

        let t1 = grouped.iter().find(|r| r.thread_id == "t1");
        let t2 = grouped.iter().find(|r| r.thread_id == "t2");
        assert!(t1.is_some());
        assert!(t2.is_some());

        let t1 = t1.expect("t1 should exist");
        assert_eq!(t1.rank, 5.0);
        assert_eq!(t1.subject.as_deref(), Some("B"));
        assert!(matches!(t1.match_kind, Some(search::MatchKind::Body)));

        let t2 = t2.expect("t2 should exist");
        assert_eq!(t2.rank, 1.0);
    }

    // -- enrichment --

    #[test]
    fn enrich_overrides_display_fields_with_latest_message_metadata() {
        let thread = DbThread {
            id: "t1".to_owned(),
            account_id: "acc1".to_owned(),
            subject: Some("Latest subject".to_owned()),
            snippet: Some("Latest snippet".to_owned()),
            last_message_at: Some(1_700_000_000),
            message_count: 5,
            is_read: true,
            is_starred: true,
            is_important: false,
            has_attachments: true,
            is_snoozed: false,
            snooze_until: None,
            is_pinned: false,
            is_muted: false,
            is_flag_complete: false,
            from_name: Some("Latest sender".to_owned()),
            from_address: Some("latest@test.com".to_owned()),
            to_addresses: None,
        };
        // Tantivy's hit may be on the best-scoring (not latest) message.
        let result = UnifiedSearchResult {
            thread_id: "t1".to_owned(),
            account_id: "acc1".to_owned(),
            subject: Some("Best-match subject".to_owned()),
            snippet: Some("Best-match snippet".to_owned()),
            from_name: Some("Best-match sender".to_owned()),
            from_address: Some("bestmatch@test.com".to_owned()),
            date: Some(1_600_000_000),
            is_read: false,
            is_starred: false,
            has_attachments: false,
            message_count: None,
            rank: 3.5,
            match_kind: Some(MatchKind::Body),
            also_matched: Vec::new(),
        };

        let enriched = enrich_from_sql(result, &thread);

        // Display fields come from the thread's latest-message metadata.
        assert_eq!(enriched.subject.as_deref(), Some("Latest subject"));
        assert_eq!(enriched.snippet.as_deref(), Some("Latest snippet"));
        assert_eq!(enriched.from_name.as_deref(), Some("Latest sender"));
        assert_eq!(enriched.from_address.as_deref(), Some("latest@test.com"));
        assert_eq!(enriched.date, Some(1_700_000_000));
        // Thread flags and counts come from the SQL thread row.
        assert!(enriched.is_read);
        assert!(enriched.is_starred);
        assert!(enriched.has_attachments);
        assert_eq!(enriched.message_count, Some(5));
        // Ranking metadata stays from Tantivy.
        assert_eq!(enriched.rank, 3.5);
        assert!(matches!(enriched.match_kind, Some(MatchKind::Body)));
    }

    // -- match preview --

    fn result_shown_as(thread_id: &str, from_name: &str, snippet: &str) -> UnifiedSearchResult {
        UnifiedSearchResult {
            thread_id: thread_id.to_owned(),
            account_id: "acc1".to_owned(),
            subject: Some("Setup complete".to_owned()),
            snippet: Some(snippet.to_owned()),
            from_name: Some(from_name.to_owned()),
            from_address: Some("notices@test.com".to_owned()),
            date: Some(1_700_000_000),
            is_read: true,
            is_starred: false,
            has_attachments: false,
            message_count: Some(1),
            rank: 1.0,
            match_kind: Some(MatchKind::Body),
            also_matched: Vec::new(),
        }
    }

    #[test]
    fn a_row_showing_no_trace_of_the_word_previews_the_line_that_holds_it() {
        let mut results = [
            result_shown_as("deep", "Cloud notices", "Health event"),
            result_shown_as("sender", "Meta for Business", "A new setting"),
            result_shown_as("opening", "Bank", "Your Metal plan renews"),
        ];
        let line = "\u{2026}Message metadata Affected account".to_owned();
        let excerpts: MatchExcerpts = ["deep", "sender", "opening"]
            .into_iter()
            .map(|thread_id| (("acc1".to_owned(), thread_id.to_owned()), line.clone()))
            .collect();

        preview_matches(&mut results, &excerpts, "meta");

        assert_eq!(results[0].snippet.as_deref(), Some(line.as_str()));
        // The word is already on the row: in the sender, in the preview.
        assert_eq!(results[1].snippet.as_deref(), Some("A new setting"));
        assert_eq!(
            results[2].snippet.as_deref(),
            Some("Your Metal plan renews")
        );
    }

    // -- parse_date_string --

    #[test]
    fn parse_date_string_unix_timestamp() {
        assert_eq!(parse_date_string("1700000000"), Some(1_700_000_000));
    }

    #[test]
    fn parse_date_string_non_numeric() {
        assert_eq!(parse_date_string("not-a-number"), None);
    }
}
