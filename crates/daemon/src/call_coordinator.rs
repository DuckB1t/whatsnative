//! The one daemon-wide call owner.
//!
//! Each account keeps its own [`CallState`](oxidezap_core::CallState) for its
//! front ends. This module only answers one other question: which local
//! account owns the call that holds the daemon's one session-wide call slot.
//! The answer is deliberately independent of sessions, devices and tasks, so
//! it can be decided and tested with made-up ids and no hardware.
//!
//! The first claim wins. A claim from a different account while that owner is
//! held is refused, with the owner named in the reason. Another call on the
//! account that already owns the slot is not a second global owner: its
//! account-local state parks it, and this module leaves the owner alone.
//!
//! [`CallPermit`] releases the owner when the runtime that claimed it drops.
//! Ends, refusals and runtime removal release by id or by account as well;
//! every release is conditional on still matching the owner, so a late end
//! cannot clear a different call that has taken the slot since.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use oxidezap_core::{AccountId, CallId};
use tokio::sync::watch;

/// Which account and call hold the one daemon-wide slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallOwner {
    pub(crate) account: AccountId,
    pub(crate) call_id: CallId,
}

impl CallOwner {
    fn new(account: AccountId, call_id: CallId) -> Self {
        Self { account, call_id }
    }
}

/// Why a claim was refused. The owner is part of the answer so the caller can
/// say which account has to end its call before this one can start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallRefusal {
    owner: CallOwner,
}

impl CallRefusal {
    #[cfg(test)]
    pub(crate) fn owner(&self) -> &CallOwner {
        &self.owner
    }
}

impl fmt::Display for CallRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a call is already in progress on account {}; end it before starting another",
            self.owner.account
        )
    }
}

impl std::error::Error for CallRefusal {}

/// A token, not an id, identifies a permit. A call id changes when the server
/// renames an outgoing placeholder, and the permit must still release it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Token(u64);

/// The pure transition table. No channel, task or device is reached here.
#[derive(Debug, Default, PartialEq, Eq)]
struct CoordinatorState {
    owner: Option<(CallOwner, Token)>,
    next: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admission {
    Held(Token),
    Existing,
}

impl CoordinatorState {
    fn claim(&mut self, account: AccountId, call_id: CallId) -> Result<Admission, CallRefusal> {
        if let Some((owner, _token)) = &self.owner {
            return if owner.account == account {
                // The account's own state parks or redelivers this offer. It
                // does not get a second permit, so dropping that parked card
                // cannot release the call still on the stage.
                Ok(Admission::Existing)
            } else {
                Err(CallRefusal {
                    owner: owner.clone(),
                })
            };
        }
        let token = Token(self.next.saturating_add(1));
        self.next = token.0;
        self.owner = Some((CallOwner::new(account, call_id), token));
        Ok(Admission::Held(token))
    }

    fn release_call(&mut self, account: AccountId, call_id: &CallId) -> bool {
        let matches = self
            .owner
            .as_ref()
            .is_some_and(|(owner, _)| owner.account == account && owner.call_id == *call_id);
        if matches {
            self.owner = None;
        }
        matches
    }

    fn release_token(&mut self, token: Token) -> bool {
        let matches = self.owner.as_ref().is_some_and(|(_, held)| *held == token);
        if matches {
            self.owner = None;
        }
        matches
    }

    fn release_account(&mut self, account: AccountId) -> bool {
        let matches = self
            .owner
            .as_ref()
            .is_some_and(|(owner, _)| owner.account == account);
        if matches {
            self.owner = None;
        }
        matches
    }

    fn rename(&mut self, account: AccountId, from: &CallId, to: CallId) -> bool {
        if self
            .owner
            .as_ref()
            .is_some_and(|(owner, _)| owner.account == account && owner.call_id == *from)
        {
            let (owner, token) = self.owner.take().expect("the owner was just matched");
            self.owner = Some((CallOwner::new(owner.account, to), token));
            true
        } else {
            false
        }
    }

    fn owner(&self) -> Option<CallOwner> {
        self.owner.as_ref().map(|(owner, _)| owner.clone())
    }
}

/// The daemon-wide slot, shared by every account runtime.
#[derive(Debug)]
pub(crate) struct CallCoordinator {
    state: Mutex<CoordinatorState>,
    /// A mirror of the owner for the tray. The decision above is pure; this
    /// only tells a follower that the value it would read has moved.
    owner: watch::Sender<Option<CallOwner>>,
}

impl CallCoordinator {
    pub(crate) fn new() -> Arc<Self> {
        let (owner, _) = watch::channel(None);
        Arc::new(Self {
            state: Mutex::new(CoordinatorState::default()),
            owner,
        })
    }

    fn lock(&self) -> MutexGuard<'_, CoordinatorState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn publish(&self, owner: Option<CallOwner>) {
        self.owner.send_if_modified(|previous| {
            if *previous == owner {
                false
            } else {
                *previous = owner;
                true
            }
        });
    }

    /// Claim the slot for one call. `None` means this account already owns it
    /// and its own call state handles the extra offer.
    pub(crate) fn claim(
        self: &Arc<Self>,
        account: AccountId,
        call_id: CallId,
    ) -> Result<Option<CallPermit>, CallRefusal> {
        let admission = {
            let mut state = self.lock();
            state.claim(account, call_id)?
        };
        self.publish(self.owner());
        let token = match admission {
            Admission::Held(token) => token,
            Admission::Existing => return Ok(None),
        };
        Ok(Some(CallPermit {
            coordinator: Arc::clone(self),
            token,
        }))
    }

    /// Give the slot up when a named call ends. A stale id changes nothing.
    pub(crate) fn release_call(&self, account: AccountId, call_id: &CallId) -> bool {
        let changed = self.lock().release_call(account, call_id);
        if changed {
            self.publish(self.owner());
        }
        changed
    }

    /// Give the slot up when the runtime that owned it goes away.
    pub(crate) fn release_account(&self, account: AccountId) -> bool {
        let changed = self.lock().release_account(account);
        if changed {
            self.publish(self.owner());
        }
        changed
    }

    fn release_token(&self, token: Token) {
        let changed = self.lock().release_token(token);
        if changed {
            self.publish(self.owner());
        }
    }

    /// A server-renamed outgoing call keeps the same permit and token.
    pub(crate) fn rename(&self, account: AccountId, from: &CallId, to: CallId) {
        let changed = self.lock().rename(account, from, to);
        if changed {
            self.publish(self.owner());
        }
    }

    pub(crate) fn owner(&self) -> Option<CallOwner> {
        self.lock().owner()
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.owner().is_some()
    }

    /// The tray is native only, so a page has no reader for this mirror.
    /// Tests still need it on wasm to prove the mirror follows the slot.
    #[cfg(any(test, not(target_family = "wasm")))]
    pub(crate) fn watch_owner(&self) -> watch::Receiver<Option<CallOwner>> {
        self.owner.subscribe()
    }
}

/// Owns one claim until the runtime holding it ends.
#[derive(Debug)]
pub(crate) struct CallPermit {
    coordinator: Arc<CallCoordinator>,
    token: Token,
}

impl Drop for CallPermit {
    fn drop(&mut self) {
        self.coordinator.release_token(self.token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: i32) -> AccountId {
        AccountId::new(value).expect("a synthetic positive account id")
    }

    fn owner(state: &CoordinatorState) -> Option<CallOwner> {
        state.owner()
    }

    #[test]
    fn the_first_claim_wins_and_a_different_account_is_told_why() {
        let mut state = CoordinatorState::default();
        assert_eq!(
            state.claim(id(11), "call-one".into()),
            Ok(Admission::Held(Token(1)))
        );
        let refusal = state.claim(id(22), "call-two".into()).unwrap_err();
        assert_eq!(refusal.owner(), &CallOwner::new(id(11), "call-one".into()));
        assert!(refusal.to_string().contains("account 11"));
        assert_eq!(
            owner(&state),
            Some(CallOwner::new(id(11), "call-one".into())),
            "a refused claim never changes the owner"
        );
        assert!(!state.release_call(id(22), &"call-two".into()));
        assert!(
            owner(&state).is_some(),
            "a release for a call that never owned the slot is inert"
        );
        assert!(state.release_call(id(11), &"call-one".into()));
        assert_eq!(owner(&state), None);
    }

    #[test]
    fn a_second_call_on_the_owner_account_does_not_take_the_slot() {
        let mut state = CoordinatorState::default();
        assert_eq!(
            state.claim(id(11), "call-one".into()),
            Ok(Admission::Held(Token(1)))
        );
        assert_eq!(
            state.claim(id(11), "call-parked".into()),
            Ok(Admission::Existing),
            "a parked offer belongs to the account-local state, not a second permit"
        );
        assert_eq!(
            owner(&state),
            Some(CallOwner::new(id(11), "call-one".into()))
        );
        assert!(!state.release_call(id(11), &"call-parked".into()));
        assert!(owner(&state).is_some());
        assert!(state.release_call(id(11), &"call-one".into()));
        assert_eq!(owner(&state), None);
    }

    #[test]
    fn rename_keeps_the_owner_and_a_stale_release_cannot_clear_it() {
        let mut state = CoordinatorState::default();
        let Admission::Held(token) = state.claim(id(11), "placeholder".into()).unwrap() else {
            panic!("a fresh slot is held by a permit");
        };
        assert!(state.rename(id(11), &"placeholder".into(), "call-real".into()));
        assert_eq!(
            owner(&state),
            Some(CallOwner::new(id(11), "call-real".into()))
        );
        assert!(!state.release_call(id(11), &"placeholder".into()));
        assert!(state.release_token(token));
        assert_eq!(owner(&state), None);
    }

    #[test]
    fn runtime_removal_releases_only_its_own_account() {
        let mut state = CoordinatorState::default();
        let _ = state.claim(id(11), "call-one".into()).unwrap();
        assert!(!state.release_account(id(22)));
        assert!(owner(&state).is_some());
        assert!(state.release_account(id(11)));
        assert_eq!(owner(&state), None);
    }

    #[test]
    fn dropping_a_permit_releases_the_slot_once() {
        let coordinator = CallCoordinator::new();
        let permit = coordinator.claim(id(11), "call-one".into()).unwrap();
        assert!(permit.is_some());
        assert!(coordinator.is_busy());
        let stale = CallPermit {
            coordinator: Arc::clone(&coordinator),
            token: Token(999),
        };
        drop(stale);
        assert!(coordinator.is_busy(), "an unknown token owns nothing");
        drop(permit);
        assert!(!coordinator.is_busy());
        coordinator.release_call(id(11), &"call-one".into());
        assert!(!coordinator.is_busy());
    }

    #[test]
    fn the_owner_watch_follows_claims_and_releases() {
        let coordinator = CallCoordinator::new();
        let mut watch = coordinator.watch_owner();
        watch.borrow_and_update();
        let permit = coordinator.claim(id(11), "call-one".into()).unwrap();
        assert!(watch.has_changed().unwrap());
        assert_eq!(
            watch.borrow_and_update().as_ref(),
            Some(&CallOwner::new(id(11), "call-one".into()))
        );
        coordinator.rename(id(11), &"call-one".into(), "call-real".into());
        assert!(watch.has_changed().unwrap());
        drop(permit);
        assert!(watch.has_changed().unwrap());
        assert_eq!(*watch.borrow_and_update(), None);
        coordinator.release_account(id(11));
        assert!(!watch.has_changed().unwrap());
    }
}
