-- description: Gmail initial sync imports attachment metadata from thread raw bytes
-- expected: pass
-- fixture: jmap-attach.toml
-- protocol: gmail
-- ceiling: 120s

local function attachment_by_filename(attachments, filename)
    for _, attachment in ipairs(attachments) do
        if attachment.filename == filename then
            return attachment
        end
    end
    return nil
end

local function mint_token(token_url)
    local response = harness.http_json({
        method = "POST",
        url = token_url,
        body = {
            grant_type = "authorization_code",
            account_id = "account-1",
            code = "harness-gmail-attachment-account-1",
            client_id = "ratatoskr-gmail-harness",
            redirect_uri = "http://127.0.0.1/oauth-callback",
        },
    })
    harness.assert(response.access_token ~= nil, "/oauth/token did not return access_token")
    return response.access_token
end

local admin_endpoint = harness.env("COURLY_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local token_url = harness.join_url(admin_endpoint, "oauth/token")
harness.clear_mock_requests(admin_endpoint)
local access_token = mint_token(token_url)

local dir = harness.data_dir("sync_gmail_attachment_initial")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-gmail-attachment@example.test",
    display_name = "Sync Gmail Attachment",
    account_name = "Sync Gmail Attachment",
    provider = "gmail_api",
    access_token = access_token,
    refresh_token = "gmail-attachment-refresh-unused",
    token_expires_at = 2000000000,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-harness",
    oauth_token_url = token_url,
})
harness.assert(account_err == nil, "TestSeedAccount failed")

harness.marker("SYNC_START")
local completed, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.marker("SYNC_END")
harness.assert(sync_err == nil, "start_sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "sync result")

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
    attachment_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.message_count, 1, "message count")
harness.assert_eq(state.attachment_count, 1, "attachment count")

local attachment = attachment_by_filename(state.attachments, "sample.txt")
harness.assert(attachment ~= nil, "missing sample.txt attachment")
harness.assert_eq(attachment.mime_type, "text/plain", "attachment mime type")
harness.assert((attachment.size or 0) > 0, "attachment size")

local requests = harness.mock_requests(admin_endpoint, { stable = true })
local message_get_requests =
    harness.request_count_prefix(requests, "gmail", "GET /gmail/v1/users/me/messages/")
harness.assert(message_get_requests >= 1, "Gmail attachment sync did not fetch a message")

harness.write_summary({
    correct = 1,
    message_count = state.message_count,
    attachment_count = state.attachment_count,
    provider_requests = #requests,
    gmail_message_get_requests = message_get_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
