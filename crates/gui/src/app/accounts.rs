//! The one account this window is connected to.
//!
//! The daemon keeps every account runtime alive. This window owns only one
//! connection to one of them: a `Session`, the task draining its events, and
//! the id the connection was opened for. Keeping those three together is what
//! lets a switch replace the connection without touching the control lane or
//! leaving an old reader attached to a new view.
//!
//! The helpers below are pure on purpose. Choosing which account to show when
//! a snapshot arrives, and undoing a choice whose attach failed, are facts
//! about ids and snapshots. They can be tested without opening a socket,
//! spawning a task, or building a window.

use gpui::Task;
use oxidezap_core::AccountId;
use oxidezap_ipc::{AccountStatus, AccountsSnapshot};

use crate::session::Session;

/// One account's connection and the reader that feeds this window from it.
///
/// `event_task` is declared before `session` so a dropped workspace stops its
/// reader before it closes the connection. A reader parked on a full queue can
/// otherwise be waiting on the very thread that is dropping the connection.
pub(super) struct AccountWorkspace {
    account: AccountId,
    event_task: Option<Task<()>>,
    session: Session,
}

impl AccountWorkspace {
    pub(super) fn new(account: AccountId, session: Session, event_task: Task<()>) -> Self {
        Self {
            account,
            event_task: Some(event_task),
            session,
        }
    }

    /// A workspace installed by a test that only needs to send requests.
    #[cfg(test)]
    pub(super) fn without_reader(account: AccountId, session: Session) -> Self {
        Self {
            account,
            event_task: None,
            session,
        }
    }

    pub(super) fn account(&self) -> AccountId {
        self.account
    }

    pub(super) fn session(&self) -> &Session {
        &self.session
    }

    /// Take the connection back, ending the event reader first.
    pub(super) fn into_session(mut self) -> Session {
        self.event_task.take();
        self.session
    }
}

/// Which account a snapshot says this window should show.
///
/// The persisted choice wins while it is still present. A persisted id that
/// has gone is not a reason to keep showing an empty account: the first id in
/// the daemon's own order is the fallback, and an empty snapshot has no honest
/// answer at all.
pub(super) fn active_in(snapshot: &AccountsSnapshot, preferred: AccountId) -> Option<AccountId> {
    if snapshot
        .accounts
        .iter()
        .any(|account| account.id == preferred)
    {
        return Some(preferred);
    }
    snapshot.accounts.first().map(|account| account.id)
}

/// The account to use after `gone` disappears from a fresh snapshot.
///
/// `None` means either that the account is still there, so nothing changed,
/// or that there is no account left to choose. The caller tells those apart by
/// looking at the snapshot itself.
pub(super) fn fallback_after_gone(
    snapshot: &AccountsSnapshot,
    gone: AccountId,
) -> Option<AccountId> {
    if !is_gone(snapshot, gone) {
        return None;
    }
    snapshot.accounts.first().map(|account| account.id)
}

/// Whether `gone` is absent from `snapshot`, with or without a successor.
///
/// An empty snapshot is the case that has no successor to attach: the window
/// must still stop showing the account the daemon has removed, and still save
/// the choice it is leaving in place for the next Add.
pub(super) fn is_gone(snapshot: &AccountsSnapshot, gone: AccountId) -> bool {
    !snapshot.accounts.iter().any(|account| account.id == gone)
}

/// The selection a failed attach has to put back.
///
/// Only an attempted switch that is still the active selection is undone. A
/// later switch may have won the race, and restoring this one then would move
/// the window away from the account it just attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SwitchIntent {
    pub previous: AccountId,
    pub requested: AccountId,
}

pub(super) fn rollback(active: &mut AccountId, intent: SwitchIntent) -> bool {
    if *active != intent.requested {
        return false;
    }
    *active = intent.previous;
    true
}

/// The translated label for a lifecycle status, without inventing a name for
/// the account behind it.
pub(crate) fn status_key(status: AccountStatus) -> crate::i18n::TextKey {
    use crate::i18n::TextKey as Key;
    use oxidezap_ipc::AccountStatus as Status;
    match status {
        Status::Starting => Key::AccountStatusStarting,
        Status::Running => Key::AccountStatusRunning,
        Status::Stopping => Key::AccountStatusStopping,
        Status::Resetting => Key::AccountStatusResetting,
        Status::Removing => Key::AccountStatusRemoving,
        Status::Stopped => Key::AccountStatusStopped,
        Status::Error => Key::AccountStatusError,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxidezap_ipc::AccountOverview;

    fn id(value: i32) -> AccountId {
        AccountId::new(value).expect("a positive synthetic id")
    }

    fn snapshot(entries: &[(i32, AccountStatus)]) -> AccountsSnapshot {
        AccountsSnapshot {
            accounts: entries
                .iter()
                .map(|(value, status)| AccountOverview {
                    id: id(*value),
                    status: *status,
                })
                .collect(),
        }
    }

    #[test]
    fn a_present_preference_is_kept_and_a_missing_one_falls_back() {
        let accounts = snapshot(&[(2, AccountStatus::Running), (1, AccountStatus::Running)]);
        assert_eq!(active_in(&accounts, id(1)), Some(id(1)));
        assert_eq!(active_in(&accounts, id(9)), Some(id(2)));
        assert_eq!(active_in(&snapshot(&[]), id(1)), None);
    }

    #[test]
    fn a_removed_active_account_falls_back_to_the_snapshot() {
        let accounts = snapshot(&[(2, AccountStatus::Running), (3, AccountStatus::Starting)]);
        assert_eq!(fallback_after_gone(&accounts, id(1)), Some(id(2)));
        assert_eq!(fallback_after_gone(&accounts, id(2)), None);
        assert_eq!(fallback_after_gone(&snapshot(&[]), id(1)), None);
    }

    #[test]
    fn a_gone_account_is_known_even_when_the_snapshot_is_empty() {
        let accounts = snapshot(&[(2, AccountStatus::Running)]);
        assert!(!is_gone(&accounts, id(2)));
        assert!(is_gone(&accounts, id(1)));
        assert!(is_gone(&snapshot(&[]), id(1)));
        assert_eq!(fallback_after_gone(&snapshot(&[]), id(1)), None);
    }

    #[test]
    fn a_failed_switch_only_rolls_back_its_own_choice() {
        let intent = SwitchIntent {
            previous: id(1),
            requested: id(2),
        };
        let mut active = id(2);
        assert!(rollback(&mut active, intent));
        assert_eq!(active, id(1));

        let mut moved = id(3);
        assert!(!rollback(&mut moved, intent));
        assert_eq!(moved, id(3));
    }

    #[test]
    fn every_lifecycle_status_has_its_own_label() {
        use crate::i18n::{Language, TextKey};
        let cases = [
            (AccountStatus::Starting, TextKey::AccountStatusStarting),
            (AccountStatus::Running, TextKey::AccountStatusRunning),
            (AccountStatus::Stopping, TextKey::AccountStatusStopping),
            (AccountStatus::Resetting, TextKey::AccountStatusResetting),
            (AccountStatus::Removing, TextKey::AccountStatusRemoving),
            (AccountStatus::Stopped, TextKey::AccountStatusStopped),
            (AccountStatus::Error, TextKey::AccountStatusError),
        ];
        for (status, key) in cases {
            assert_eq!(status_key(status) as u8, key as u8);
            for language in Language::ALL {
                assert!(!language.text(key).is_empty());
            }
        }
    }
}
