//! Read-only account selection for instance inventories. Never opens secrets,
//! refreshes authentication or performs network requests.
use std::path::PathBuf;

use serde::Serialize;
use uuid::Uuid;

use super::{
    AccountAuthenticationState, AccountMetadata, AccountProvider, AccountRepository,
    account_avatar_path,
};
use crate::instance::{InstanceKind, InstanceManifest};
use crate::runtime::RuntimePaths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccountSelectionSource {
    Instance,
    Global,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountPresentation {
    pub id: Uuid,
    pub profile_name: String,
    pub provider: AccountProvider,
    pub authentication_state: AccountAuthenticationState,
    pub avatar_path: Option<PathBuf>,
}

impl AccountPresentation {
    pub fn new(paths: &RuntimePaths, account: &AccountMetadata) -> Self {
        Self {
            id: account.id,
            profile_name: account.profile_name.clone(),
            provider: account.provider.clone(),
            authentication_state: account.authentication_state,
            avatar_path: account_avatar_path(paths, account).filter(|path| path.is_file()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum AccountSelectionSummary {
    NotConfigured,
    Unavailable {
        account_id: Uuid,
        source: AccountSelectionSource,
    },
    Selected {
        account: AccountPresentation,
        source: AccountSelectionSource,
    },
}

impl AccountRepository {
    pub fn instance_selection_summary(
        &self,
        paths: &RuntimePaths,
        manifest: &InstanceManifest,
    ) -> Option<AccountSelectionSummary> {
        (manifest.kind == InstanceKind::Client)
            .then(|| self.selection_summary(paths, manifest.launch.account))
    }
    pub fn selection_summary(
        &self,
        paths: &RuntimePaths,
        explicit: Option<Uuid>,
    ) -> AccountSelectionSummary {
        let Some(account_id) = self.selection_id(explicit) else {
            return AccountSelectionSummary::NotConfigured;
        };
        let source = if explicit.is_some() {
            AccountSelectionSource::Instance
        } else {
            AccountSelectionSource::Global
        };
        match self
            .accounts()
            .iter()
            .find(|account| account.id == account_id)
        {
            Some(account) => AccountSelectionSummary::Selected {
                account: AccountPresentation::new(paths, account),
                source,
            },
            None => AccountSelectionSummary::Unavailable { account_id, source },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::create_offline_account;
    use crate::instance::LoaderKind;
    use crate::runtime::RuntimePathOptions;

    #[test]
    fn inventory_selection_uses_launch_precedence_without_authenticating_or_falling_back() {
        let temp = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::resolve(&RuntimePathOptions {
            config_dir: Some(temp.path().join("config")),
            data_dir: Some(temp.path().join("data")),
            cache_dir: Some(temp.path().join("cache")),
        })
        .unwrap();
        assert!(matches!(
            AccountRepository::load(&paths)
                .unwrap()
                .selection_summary(&paths, None),
            AccountSelectionSummary::NotConfigured
        ));
        let default = create_offline_account(&paths, "Default").unwrap();
        let explicit = create_offline_account(&paths, "Explicit").unwrap();
        let repository = AccountRepository::load(&paths).unwrap();
        for (selected, expected, source) in [
            (None, default.id, AccountSelectionSource::Global),
            (
                Some(explicit.id),
                explicit.id,
                AccountSelectionSource::Instance,
            ),
        ] {
            let AccountSelectionSummary::Selected {
                account,
                source: actual,
            } = repository.selection_summary(&paths, selected)
            else {
                panic!("missing account");
            };
            assert_eq!(account.id, repository.selected(selected).unwrap().id);
            assert_eq!(account.id, expected);
            assert_eq!(actual, source);
            let json = serde_json::to_string(&account).unwrap();
            assert!(!json.contains("token"));
            assert!(!json.contains("login_name"));
        }
        let missing = Uuid::new_v4();
        assert!(
            matches!(repository.selection_summary(&paths, Some(missing)), AccountSelectionSummary::Unavailable { account_id, source: AccountSelectionSource::Instance } if account_id == missing)
        );
        assert!(repository.selected(Some(missing)).is_err());
        let server = InstanceManifest::new(
            Uuid::new_v4(),
            "Server",
            InstanceKind::Server,
            "26.2",
            LoaderKind::Vanilla,
            None,
        )
        .unwrap();
        assert!(
            repository
                .instance_selection_summary(&paths, &server)
                .is_none()
        );
    }
}
