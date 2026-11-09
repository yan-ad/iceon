use serde::Serialize;

pub const REPOSITORY_URL: &str = "https://github.com/yan-ad/iceon";
pub const HOMEPAGE_URL: &str = "https://iceon.yan.ad";

pub fn open_homepage_with<E>(open: impl FnOnce(&str) -> Result<(), E>) -> Result<(), ProjectError> {
    open(HOMEPAGE_URL).map_err(|_| ProjectError::OpenFailed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectError {
    OpenFailed,
}

pub fn open_repository_with<E>(
    open: impl FnOnce(&str) -> Result<(), E>,
) -> Result<(), ProjectError> {
    open(REPOSITORY_URL).map_err(|_| ProjectError::OpenFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homepage_has_no_caller_supplied_destination() {
        let mut calls = Vec::new();
        open_homepage_with(|url| {
            calls.push(url.to_owned());
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(calls, ["https://iceon.yan.ad"]);
        assert_eq!(
            open_homepage_with(|_| Err("private diagnostic")),
            Err(ProjectError::OpenFailed)
        );
    }

    #[test]
    fn opens_only_the_fixed_https_repository() {
        let mut calls = Vec::new();
        assert_eq!(
            open_repository_with(|url| {
                calls.push(url.to_owned());
                Ok::<_, ()>(())
            }),
            Ok(())
        );
        assert_eq!(calls, ["https://github.com/yan-ad/iceon"]);
        let url = url::Url::parse(REPOSITORY_URL).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("github.com"));
        assert!(url.username().is_empty());
        assert!(url.password().is_none());
        assert!(url.query().is_none());
        assert!(url.fragment().is_none());
    }

    #[test]
    fn opener_failures_do_not_expose_platform_details() {
        let error = open_repository_with(|_| Err("private platform diagnostics")).unwrap_err();
        assert_eq!(serde_json::to_string(&error).unwrap(), "\"openFailed\"");
    }
}
