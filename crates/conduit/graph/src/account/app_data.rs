//! Application documents kept in the mailbox itself.
//!
//! Exchange has no settings store an application can call its own through
//! Graph, so the documents live where Outlook keeps its own roaming
//! settings: in the mailbox, out of the user's sight. One hidden top-level
//! mail folder holds one message per document. The message's subject is the
//! document's key and its plain-text body is the document, base64-encoded so
//! Exchange's body normalisation (line wrapping, trailing newline, charset
//! rewriting) cannot alter a byte of it.
//!
//! A set is find-then-write. Two clients creating the same key at the same
//! moment each file a message; the next read takes the most recently
//! modified one and the next set removes the others.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use conduit_types::{AccountError, AccountOperation};
use serde::Deserialize;
use serde_json::json;

use super::GraphAccount;
use super::graph_error::{GraphErrorContext, into_account_error};
use crate::error::GraphError;

/// Display name of the hidden folder. Hidden folders stay out of every
/// Outlook folder list; the name only shows in administrative tools.
const FOLDER_NAME: &str = "Courly";

/// Top-level folders read in one page when looking for the hidden one.
const FOLDER_PAGE: usize = 250;

/// Documents read in one page. The store holds a handful of them.
const DOCUMENT_PAGE: usize = 100;

/// Largest document stored (bytes). Graph takes a message of about 4 MB in
/// one request, and base64 grows the document by a third.
const MAX_DOCUMENT_BYTES: usize = 2 * 1024 * 1024;

const TEXT_BODY: &str = "outlook.body-content-type=\"text\"";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Folder {
    id: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    is_hidden: bool,
}

#[derive(Debug, Deserialize)]
struct FolderPage {
    value: Vec<Folder>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Document {
    id: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    last_modified_date_time: String,
    body: Option<DocumentBody>,
}

#[derive(Debug, Deserialize)]
struct DocumentBody {
    #[serde(default)]
    content: String,
}

#[derive(Debug, Deserialize)]
struct DocumentPage {
    value: Vec<Document>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

fn malformed(message: String) -> GraphError {
    GraphError::Json {
        message,
        body: None,
    }
}

/// The hidden folder's id, or `None` when no client has written yet.
async fn find_folder(account: &GraphAccount) -> Result<Option<String>, GraphError> {
    let prefix = account.client.api_path_prefix();
    let page: FolderPage = account
        .client
        .get_json(&format!(
            "{prefix}/mailFolders?includeHiddenFolders=true&$select=id,displayName,isHidden&$top={FOLDER_PAGE}"
        ))
        .await?;
    let found = page
        .value
        .into_iter()
        .find(|folder| folder.is_hidden && folder.display_name == FOLDER_NAME);
    match found {
        Some(folder) => Ok(Some(folder.id)),
        // Creating one now could file a second store beside the one a
        // later page holds.
        None if page.next_link.is_some() => Err(malformed(format!(
            "Graph mailFolders listing: more than {FOLDER_PAGE} top-level folders, the application folder cannot be told absent"
        ))),
        None => Ok(None),
    }
}

async fn create_folder(account: &GraphAccount) -> Result<String, GraphError> {
    let prefix = account.client.api_path_prefix();
    let folder: Folder = account
        .client
        .post(
            &format!("{prefix}/mailFolders"),
            &json!({ "displayName": FOLDER_NAME, "isHidden": true }),
        )
        .await?;
    if !folder.is_hidden {
        // A server that files it in plain sight would also fail to find it
        // again as the hidden store, and the next write would add another.
        account
            .client
            .delete(&format!(
                "{prefix}/mailFolders/{}",
                conduit_net::url::encode_path_component(&folder.id)
            ))
            .await?;
        return Err(malformed(
            "Graph mailFolders create: the server did not hide the application folder".to_owned(),
        ));
    }
    Ok(folder.id)
}

/// Every message filed under `key`, most recently modified first.
async fn documents(
    account: &GraphAccount,
    folder: &str,
    key: &str,
) -> Result<Vec<Document>, GraphError> {
    let prefix = account.client.api_path_prefix();
    let page: DocumentPage = account
        .client
        .get_json_prefer(
            &format!(
                "{prefix}/mailFolders/{}/messages?$select=id,subject,lastModifiedDateTime,body&$top={DOCUMENT_PAGE}",
                conduit_net::url::encode_path_component(folder)
            ),
            TEXT_BODY,
        )
        .await?;
    if page.next_link.is_some() {
        return Err(malformed(format!(
            "Graph application folder: more than {DOCUMENT_PAGE} documents"
        )));
    }
    let mut found: Vec<Document> = page
        .value
        .into_iter()
        .filter(|document| document.subject == key)
        .collect();
    // ISO 8601 UTC timestamps order as text.
    found.sort_by(|a, b| b.last_modified_date_time.cmp(&a.last_modified_date_time));
    Ok(found)
}

fn decode(document: &Document) -> Result<String, GraphError> {
    let content = document
        .body
        .as_ref()
        .map(|body| body.content.as_str())
        .unwrap_or_default();
    let packed: String = content.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = BASE64.decode(packed).map_err(|e| {
        malformed(format!(
            "Graph application document {:?}: body is not base64: {e}",
            document.subject
        ))
    })?;
    String::from_utf8(bytes).map_err(|e| {
        malformed(format!(
            "Graph application document {:?}: not UTF-8: {e}",
            document.subject
        ))
    })
}

async fn get(account: &GraphAccount, key: &str) -> Result<Option<String>, GraphError> {
    let Some(folder) = find_folder(account).await? else {
        return Ok(None);
    };
    match documents(account, &folder, key).await?.first() {
        Some(document) => decode(document).map(Some),
        None => Ok(None),
    }
}

async fn delete_document(account: &GraphAccount, id: &str) -> Result<(), GraphError> {
    let prefix = account.client.api_path_prefix();
    account
        .client
        .delete(&format!(
            "{prefix}/messages/{}",
            conduit_net::url::encode_path_component(id)
        ))
        .await
}

async fn set(account: &GraphAccount, key: &str, value: Option<&str>) -> Result<(), GraphError> {
    if let Some(value) = value
        && value.len() > MAX_DOCUMENT_BYTES
    {
        return Err(GraphError::Configuration {
            message: format!(
                "application document {key:?} is {} bytes, the mailbox store takes {MAX_DOCUMENT_BYTES}",
                value.len()
            ),
        });
    }
    let folder = match (find_folder(account).await?, value) {
        (Some(folder), _) => folder,
        (None, None) => return Ok(()),
        (None, Some(_)) => create_folder(account).await?,
    };
    let mut existing = documents(account, &folder, key).await?.into_iter();
    let prefix = account.client.api_path_prefix();
    let kept = existing.next();
    match (value, kept) {
        (Some(value), Some(document)) => {
            account
                .client
                .patch(
                    &format!(
                        "{prefix}/messages/{}",
                        conduit_net::url::encode_path_component(&document.id)
                    ),
                    &json!({ "body": body_of(value) }),
                )
                .await?;
        }
        (Some(value), None) => {
            account
                .client
                .post::<serde_json::Value, _>(
                    &format!(
                        "{prefix}/mailFolders/{}/messages",
                        conduit_net::url::encode_path_component(&folder)
                    ),
                    &json!({ "subject": key, "isRead": true, "body": body_of(value) }),
                )
                .await?;
        }
        (None, Some(document)) => delete_document(account, &document.id).await?,
        (None, None) => {}
    }
    // Copies a concurrent first write left behind.
    for duplicate in existing {
        delete_document(account, &duplicate.id).await?;
    }
    Ok(())
}

fn body_of(value: &str) -> serde_json::Value {
    json!({ "contentType": "text", "content": BASE64.encode(value) })
}

pub(crate) async fn app_data_get(
    account: GraphAccount,
    key: String,
) -> Result<Option<String>, AccountError> {
    get(&account, &key)
        .await
        .map_err(|e| into_account_error(e, GraphErrorContext::graph(AccountOperation::AppDataGet)))
}

pub(crate) async fn app_data_set(
    account: GraphAccount,
    key: String,
    value: Option<String>,
) -> Result<(), AccountError> {
    set(&account, &key, value.as_deref())
        .await
        .map_err(|e| into_account_error(e, GraphErrorContext::graph(AccountOperation::AppDataSet)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use conduit_net::test_support::{Canned, RequestSnapshot, ScriptedDispatch};
    use std::sync::Arc;

    fn scripted(steps: Vec<Canned>) -> (GraphAccount, Arc<ScriptedDispatch>) {
        use conduit_net::test_support::scripted_account;
        use conduit_net::{NetConfig, RetryPolicy, StaticTokenSource, TokenSource};

        let script = ScriptedDispatch::new(steps);
        let token_source: Arc<dyn TokenSource> = Arc::new(StaticTokenSource::new("token", None));
        let net = scripted_account(
            &script,
            NetConfig::default(),
            Vec::new(),
            Arc::clone(&token_source),
            RetryPolicy::disabled(),
        );
        let client = crate::client::GraphClient::with_account_net(
            net,
            "https://graph.contoso.test/v1.0",
            token_source,
        );
        let account =
            GraphAccount::new_for_tests(client, crate::account::PushMode::GraphSubscriptions);
        (account, script)
    }

    fn response(status: reqwest::StatusCode, body: serde_json::Value) -> Canned {
        Canned::Response {
            status,
            headers: reqwest::header::HeaderMap::new(),
            body: bytes::Bytes::from(body.to_string()),
        }
    }

    fn ok(body: serde_json::Value) -> Canned {
        response(reqwest::StatusCode::OK, body)
    }

    fn no_content() -> Canned {
        Canned::Response {
            status: reqwest::StatusCode::NO_CONTENT,
            headers: reqwest::header::HeaderMap::new(),
            body: bytes::Bytes::new(),
        }
    }

    fn folders_with_store() -> serde_json::Value {
        json!({ "value": [
            { "id": "inbox", "displayName": "Inbox", "isHidden": false },
            // A visible folder the user happened to name the same.
            { "id": "visible", "displayName": "Courly", "isHidden": false },
            { "id": "store", "displayName": "Courly", "isHidden": true },
        ] })
    }

    fn folders_without_store() -> serde_json::Value {
        json!({ "value": [{ "id": "inbox", "displayName": "Inbox", "isHidden": false }] })
    }

    fn document(id: &str, key: &str, modified: &str, value: &str) -> serde_json::Value {
        json!({
            "id": id,
            "subject": key,
            "lastModifiedDateTime": modified,
            "body": { "contentType": "text", "content": BASE64.encode(value) },
        })
    }

    fn body_json(request: &RequestSnapshot) -> serde_json::Value {
        serde_json::from_slice(request.body.as_ref().expect("request body")).expect("json body")
    }

    #[tokio::test]
    async fn a_mailbox_no_client_wrote_to_has_no_document_and_stays_untouched() {
        let (account, script) = scripted(vec![ok(folders_without_store())]);
        let value = app_data_get(account, "signatures".to_string())
            .await
            .expect("get succeeds");
        assert_eq!(value, None);
        let requests = script.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, reqwest::Method::GET);
        assert!(
            requests[0]
                .url
                .query()
                .is_some_and(|query| query.contains("includeHiddenFolders=true"))
        );
    }

    #[tokio::test]
    async fn get_reads_the_hidden_folder_and_the_newest_copy_of_the_key() {
        let wrapped = {
            // Exchange hands a long text body back wrapped and newline-ended.
            let packed = BASE64.encode("{\"v\":\"é new\"}");
            let (head, tail) = packed.split_at(6);
            format!("{head}\r\n{tail}\r\n")
        };
        let (account, script) = scripted(vec![
            ok(folders_with_store()),
            ok(json!({ "value": [
                document("old", "signatures", "2026-11-01T10:00:00Z", "{\"v\":\"old\"}"),
                document("other", "rules", "2026-11-20T10:00:00Z", "{}"),
                {
                    "id": "new",
                    "subject": "signatures",
                    "lastModifiedDateTime": "2026-11-27T09:00:00Z",
                    "body": { "contentType": "text", "content": wrapped },
                },
            ] })),
        ]);
        let value = app_data_get(account, "signatures".to_string())
            .await
            .expect("get succeeds");
        assert_eq!(value.as_deref(), Some("{\"v\":\"é new\"}"));
        let requests = script.requests();
        assert!(
            requests[1]
                .url
                .path()
                .ends_with("/mailFolders/store/messages")
        );
    }

    #[tokio::test]
    async fn the_first_set_creates_the_hidden_folder_then_the_document() {
        let (account, script) = scripted(vec![
            ok(folders_without_store()),
            response(
                reqwest::StatusCode::CREATED,
                json!({ "id": "store", "displayName": "Courly", "isHidden": true }),
            ),
            ok(json!({ "value": [] })),
            response(reqwest::StatusCode::CREATED, json!({ "id": "doc" })),
        ]);
        app_data_set(
            account,
            "signatures".to_string(),
            Some("{\"a\":1}".to_string()),
        )
        .await
        .expect("set succeeds");

        let requests = script.requests();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[1].method, reqwest::Method::POST);
        assert!(requests[1].url.path().ends_with("/mailFolders"));
        assert_eq!(
            body_json(&requests[1]),
            json!({ "displayName": "Courly", "isHidden": true })
        );
        assert_eq!(requests[3].method, reqwest::Method::POST);
        assert!(
            requests[3]
                .url
                .path()
                .ends_with("/mailFolders/store/messages")
        );
        assert_eq!(
            body_json(&requests[3]),
            json!({
                "subject": "signatures",
                "isRead": true,
                "body": { "contentType": "text", "content": BASE64.encode("{\"a\":1}") },
            })
        );
    }

    #[tokio::test]
    async fn a_server_that_does_not_hide_the_folder_gets_it_removed_and_no_document() {
        let (account, script) = scripted(vec![
            ok(folders_without_store()),
            response(
                reqwest::StatusCode::CREATED,
                json!({ "id": "plain", "displayName": "Courly" }),
            ),
            no_content(),
        ]);
        let error = app_data_set(account, "signatures".to_string(), Some("1".to_string()))
            .await
            .expect_err("a visible store is refused");
        assert_eq!(error.operation(), Some(AccountOperation::AppDataSet));
        let requests = script.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2].method, reqwest::Method::DELETE);
        assert!(requests[2].url.path().ends_with("/mailFolders/plain"));
    }

    #[tokio::test]
    async fn a_document_too_large_for_one_request_is_refused_before_any_request() {
        let (account, script) = scripted(Vec::new());
        let value = "x".repeat(MAX_DOCUMENT_BYTES + 1);
        app_data_set(account, "signatures".to_string(), Some(value))
            .await
            .expect_err("an oversized document is refused");
        assert!(script.requests().is_empty());
    }

    #[tokio::test]
    async fn a_later_set_rewrites_the_newest_copy_and_removes_the_others() {
        let (account, script) = scripted(vec![
            ok(folders_with_store()),
            ok(json!({ "value": [
                document("old", "signatures", "2026-11-01T10:00:00Z", "1"),
                document("new", "signatures", "2026-11-27T09:00:00Z", "2"),
            ] })),
            no_content(),
            no_content(),
        ]);
        app_data_set(account, "signatures".to_string(), Some("3".to_string()))
            .await
            .expect("set succeeds");

        let requests = script.requests();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[2].method, reqwest::Method::PATCH);
        assert!(requests[2].url.path().ends_with("/messages/new"));
        assert_eq!(
            body_json(&requests[2]),
            json!({ "body": { "contentType": "text", "content": BASE64.encode("3") } })
        );
        assert_eq!(requests[3].method, reqwest::Method::DELETE);
        assert!(requests[3].url.path().ends_with("/messages/old"));
    }

    #[tokio::test]
    async fn clearing_a_key_deletes_its_document_and_never_creates_the_folder() {
        let (account, script) = scripted(vec![
            ok(folders_with_store()),
            ok(json!({ "value": [document("doc", "signatures", "2026-11-27T09:00:00Z", "1")] })),
            no_content(),
        ]);
        app_data_set(account, "signatures".to_string(), None)
            .await
            .expect("clear succeeds");
        let requests = script.requests();
        assert_eq!(requests[2].method, reqwest::Method::DELETE);
        assert!(requests[2].url.path().ends_with("/messages/doc"));

        let (account, script) = scripted(vec![ok(folders_without_store())]);
        app_data_set(account, "signatures".to_string(), None)
            .await
            .expect("clearing an absent store succeeds");
        assert_eq!(script.requests().len(), 1);
    }

    #[tokio::test]
    async fn a_folder_listing_that_does_not_fit_one_page_is_refused_not_guessed() {
        let (account, script) = scripted(vec![ok(json!({
            "value": [{ "id": "inbox", "displayName": "Inbox", "isHidden": false }],
            "@odata.nextLink": "https://graph.contoso.test/v1.0/me/mailFolders?$skip=250",
        }))]);
        let error = app_data_set(account, "signatures".to_string(), Some("1".to_string()))
            .await
            .expect_err("an unreadable listing must not create a second store");
        assert_eq!(error.operation(), Some(AccountOperation::AppDataSet));
        assert_eq!(script.requests().len(), 1);
    }
}
