use futures::future::BoxFuture;
use tokio_util::sync::CancellationToken;

use super::{Entry, Field, Profile, Result, vault};

pub(super) trait SecretProvider: Send + Sync {
    fn login<'a>(
        &'a self,
        profile: &'a Profile,
        username: &'a str,
        password: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<vault::Auth>>;

    fn resume<'a>(
        &'a self,
        profile: &'a Profile,
        token: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<u64>>>;

    fn list<'a>(
        &'a self,
        profile: &'a Profile,
        token: &'a str,
        path: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<(Vec<Entry>, bool)>>;

    fn reveal<'a>(
        &'a self,
        profile: &'a Profile,
        token: &'a str,
        path: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<(Vec<Field>, u64)>>;
}

pub(super) struct VaultProvider;

impl SecretProvider for VaultProvider {
    fn login<'a>(
        &'a self,
        profile: &'a Profile,
        username: &'a str,
        password: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<vault::Auth>> {
        Box::pin(async move {
            vault::Vault::new()?
                .login(profile, username, password, cancel)
                .await
        })
    }

    fn resume<'a>(
        &'a self,
        profile: &'a Profile,
        token: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<u64>>> {
        Box::pin(async move { vault::Vault::new()?.resume(profile, token, cancel).await })
    }

    fn list<'a>(
        &'a self,
        profile: &'a Profile,
        token: &'a str,
        path: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<(Vec<Entry>, bool)>> {
        Box::pin(async move {
            vault::Vault::new()?
                .list(profile, token, path, cancel)
                .await
        })
    }

    fn reveal<'a>(
        &'a self,
        profile: &'a Profile,
        token: &'a str,
        path: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<(Vec<Field>, u64)>> {
        Box::pin(async move {
            vault::Vault::new()?
                .reveal(profile, token, path, cancel)
                .await
        })
    }
}
