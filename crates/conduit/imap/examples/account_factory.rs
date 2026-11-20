use std::env;
use std::sync::Arc;

use conduit_imap::{AuthPolicy, Credentials, ImapAccountConfig, ImapAccountFactory, ImapConfig};
use conduit_types::{AccountFactory, AccountId};

#[tokio::main]
async fn main() {
    let host = env::var("CONDUIT_IMAP_HOST").expect("CONDUIT_IMAP_HOST is required");
    let username = env::var("CONDUIT_IMAP_USERNAME").expect("CONDUIT_IMAP_USERNAME is required");
    let password = env::var("CONDUIT_IMAP_PASSWORD").expect("CONDUIT_IMAP_PASSWORD is required");

    let config = ImapAccountConfig::new(
        ImapConfig::tls(host),
        Credentials::password(username, password),
        AuthPolicy::default(),
    );
    let factory: Arc<dyn AccountFactory> = Arc::new(ImapAccountFactory::new(config));
    let opened = factory
        .open(AccountId("imap-example".to_owned()))
        .await
        .expect("account opens");
    let account = opened.account;
    // Composed DAV sub-accounts that failed to open are reported here.
    let _degraded = opened.skipped_scopes;

    let _capabilities = account.capabilities();
    let containers = account.containers_list().await.expect("containers list");
    let _list = containers.containers;
    account.close().await.expect("account closes");
}
