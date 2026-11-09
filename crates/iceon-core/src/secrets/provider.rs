use futures::future::BoxFuture;
use tokio_util::sync::CancellationToken;

use super::{Entry, Field, MetadataData, Mount, Profile, Result, vault};

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
    fn mounts<'a>(
        &'a self,
        profile: &'a Profile,
        token: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<(Vec<Mount>, bool)>> {
        let _ = (profile, token, cancel);
        Box::pin(async { Err(super::Error::InvalidResponse) })
    }
    fn metadata<'a>(
        &'a self,
        profile: &'a Profile,
        token: &'a str,
        path: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<MetadataData>> {
        let _ = (profile, token, path, cancel);
        Box::pin(async { Err(super::Error::InvalidResponse) })
    }
}

pub(super) struct VaultProvider;

impl SecretProvider for VaultProvider {
    fn mounts<'a>(
        &'a self,
        profile: &'a Profile,
        token: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<(Vec<Mount>, bool)>> {
        Box::pin(async move { vault::Vault::new()?.mounts(profile, token, cancel).await })
    }
    fn metadata<'a>(
        &'a self,
        profile: &'a Profile,
        token: &'a str,
        path: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<MetadataData>> {
        Box::pin(async move {
            vault::Vault::new()?
                .metadata(profile, token, path, cancel)
                .await
        })
    }
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
