//! A daemon-wide tray follows current runtimes, including replacements and removals.
use std::sync::Arc;

use tokio::sync::{mpsc, watch};

use crate::account::AccountRegistry;
use crate::call_coordinator::CallOwner;
use crate::state::{StateHub, TrayCall, TrayState};

pub struct TrayHub {
    registry: Arc<AccountRegistry>,
    state: watch::Sender<TrayState>,
    /// The daemon-wide call marker, separate from any one account's state.
    calls: watch::Receiver<Option<CallOwner>>,
    empty: Arc<StateHub>,
}

impl TrayHub {
    pub fn new(registry: Arc<AccountRegistry>) -> Arc<Self> {
        let calls = registry.call_coordinator().watch_owner();
        let (state, _) = watch::channel(TrayState {
            connected: false,
            unread: 0,
            call: None,
        });
        Arc::new(Self {
            registry,
            state,
            calls,
            empty: StateHub::new(),
        })
    }

    pub fn registry(&self) -> Arc<AccountRegistry> {
        Arc::clone(&self.registry)
    }
    pub fn watch_tray(&self) -> watch::Receiver<TrayState> {
        self.state.subscribe()
    }

    pub fn hubs(&self) -> Vec<Arc<StateHub>> {
        self.current_hubs(true)
    }

    fn current_hubs(&self, include_closing_windows: bool) -> Vec<Arc<StateHub>> {
        let hubs: Vec<_> = self
            .registry
            .runtimes()
            .into_iter()
            .filter(|runtime| {
                !runtime.is_stopping()
                    || (include_closing_windows && runtime.hub().windows_attached())
            })
            .map(|runtime| runtime.hub())
            .collect();
        if hubs.is_empty() {
            vec![Arc::clone(&self.empty)]
        } else {
            hubs
        }
    }

    pub fn windows_attached(&self) -> bool {
        self.hubs().iter().any(|hub| hub.windows_attached())
    }
    pub fn show(&self) {
        crate::window::show_every(self.hubs());
    }
    pub fn hide(&self) {
        for hub in self.hubs() {
            crate::window::hide(&hub);
        }
    }

    fn publish(&self) {
        let values: Vec<_> = self
            .current_hubs(false)
            .iter()
            .map(|hub| hub.watch_tray().borrow().clone())
            .collect();
        let call = self.calls.borrow().as_ref().map(|owner| TrayCall {
            account: owner.account,
            call_id: owner.call_id.clone(),
        });
        let state = aggregate(&values, call);
        self.state.send_if_modified(|previous| {
            if *previous == state {
                false
            } else {
                *previous = state;
                true
            }
        });
    }

    pub async fn follow(self: Arc<Self>) {
        let mut accounts = self.registry.subscribe();
        let mut retirements = self.registry.watch_retirements();
        let mut calls = self.calls.clone();
        let (changed, mut changes) = mpsc::channel(1);
        let mut watchers = tokio::task::JoinSet::new();
        loop {
            watchers.abort_all();
            while watchers.join_next().await.is_some() {}
            accounts.borrow_and_update();
            retirements.borrow_and_update();
            calls.borrow_and_update();
            for hub in self.current_hubs(false) {
                let mut state = hub.watch_tray();
                let changed = changed.clone();
                watchers.spawn(async move {
                    while state.changed().await.is_ok() {
                        state.borrow_and_update();
                        let _ = changed.try_send(());
                    }
                });
            }
            self.publish();
            loop {
                tokio::select! {
                    result = retirements.changed() => {
                        if result.is_err() { return; }
                        break;
                    }
                    result = accounts.changed() => {
                        if result.is_err() { return; }
                        break;
                    }
                    Some(()) = changes.recv() => self.publish(),
                    result = calls.changed() => {
                        if result.is_err() { return; }
                        self.publish();
                    }
                }
            }
        }
    }
}

fn aggregate(states: &[TrayState], call: Option<TrayCall>) -> TrayState {
    TrayState {
        connected: states.iter().any(|state| state.connected),
        unread: states.iter().fold(0u32, |total, state| {
            total.saturating_add(state.shown_unread())
        }),
        call,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::AccountRuntime;
    use oxidezap_core::AccountId;
    use oxidezap_ipc::{ConnectionState, DaemonEvent};

    fn runtime(id: i32) -> Arc<AccountRuntime> {
        let id = AccountId::new(id).unwrap();
        let (commands, _receiver) = mpsc::channel(1);
        Arc::new(AccountRuntime::new(
            id,
            StateHub::for_account(id),
            Arc::new(oxidezap_plugin_host::Plugins::nothing_loaded(Arc::new(
                |_| {},
            ))),
            commands,
        ))
    }

    #[tokio::test]
    async fn removed_runtime_cannot_publish_stale_tray_state_or_own_window_actions() {
        let registry = AccountRegistry::new();
        let old = runtime(2);
        old.hub()
            .apply(crate::state::Change::live(DaemonEvent::ConnectionChanged(
                ConnectionState::Connected,
            )));
        registry.insert(Arc::clone(&old));
        let tray = TrayHub::new(Arc::clone(&registry));
        let mut state = tray.watch_tray();
        let following = tokio::spawn(Arc::clone(&tray).follow());
        tokio::time::timeout(std::time::Duration::from_secs(5), state.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(state.borrow_and_update().connected);
        registry.remove(old.id());
        tokio::time::timeout(std::time::Duration::from_secs(5), state.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(!state.borrow_and_update().connected);

        let successor = runtime(3);
        registry.insert(Arc::clone(&successor));
        let mut signals = successor.hub().subscribe_signals();
        let _window = successor.hub().attach_window();
        let _stale_window = old.hub().attach_window();
        let mut stale_signals = old.hub().subscribe_signals();
        assert!(tray.windows_attached());
        crate::window::Toggle::default().click_every(tray.hubs());
        assert!(matches!(
            serde_json::from_str::<oxidezap_ipc::DaemonMessage>(&signals.try_recv().unwrap())
                .unwrap(),
            oxidezap_ipc::DaemonMessage::HideWindow
        ));
        assert!(stale_signals.try_recv().is_err());
        tray.show();
        assert!(matches!(
            serde_json::from_str::<oxidezap_ipc::DaemonMessage>(&signals.try_recv().unwrap())
                .unwrap(),
            oxidezap_ipc::DaemonMessage::ShowWindow
        ));
        assert!(stale_signals.try_recv().is_err());
        following.abort();
    }

    fn state(connected: bool, unread: u32) -> TrayState {
        TrayState {
            connected,
            unread,
            call: None,
        }
    }

    #[test]
    fn disconnected_retained_history_does_not_hide_connected_successor() {
        assert_eq!(
            aggregate(&[state(false, 100), state(true, 7)], None),
            state(true, 7)
        );
        assert_eq!(aggregate(&[], None), state(false, 0));
        assert_eq!(
            aggregate(&[state(true, u32::MAX), state(true, 1)], None).unread,
            u32::MAX
        );
    }

    #[test]
    fn aggregate_publishes_the_one_global_call_marker() {
        let call = TrayCall {
            account: AccountId::new(4).unwrap(),
            call_id: "synthetic-call".into(),
        };
        let merged = aggregate(&[state(true, 2), state(false, 9)], Some(call.clone()));
        assert_eq!(merged.call, Some(call));
        assert!(
            merged
                .description()
                .contains("call in progress on account 4")
        );
    }

    #[tokio::test]
    async fn call_marker_follows_claims_and_runtime_removal() {
        let registry = AccountRegistry::new();
        let account = runtime(4);
        account
            .hub()
            .apply(crate::state::Change::live(DaemonEvent::ConnectionChanged(
                ConnectionState::Connected,
            )));
        registry.insert(Arc::clone(&account));
        let tray = TrayHub::new(Arc::clone(&registry));
        let mut watch = tray.watch_tray();
        let following = tokio::spawn(Arc::clone(&tray).follow());
        tokio::time::timeout(std::time::Duration::from_secs(5), watch.changed())
            .await
            .unwrap()
            .unwrap();
        watch.borrow_and_update();

        let permit = registry
            .call_coordinator()
            .claim(account.id(), "synthetic-call".into())
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), watch.changed())
            .await
            .unwrap()
            .unwrap();
        let state = watch.borrow_and_update();
        assert_eq!(state.call.as_ref().unwrap().account, account.id());
        assert_eq!(state.call.as_ref().unwrap().call_id, "synthetic-call");
        drop(state);

        registry.remove(account.id());
        tokio::time::timeout(std::time::Duration::from_secs(5), watch.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(watch.borrow_and_update().call.is_none());
        drop(permit);
        following.abort();
    }
}
