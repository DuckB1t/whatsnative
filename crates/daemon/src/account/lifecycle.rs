//! Account lifecycle for an embedded daemon host, including a browser page.
//!
//! [`super::AccountSupervisor`] owns a `tokio::task::JoinSet`, which asks
//! every task it holds to be `Send`. A page's plugin host is built from
//! `wasm-bindgen` closures that are deliberately not, so a supervisor cannot
//! be built there. This is the other path: it keeps the same rules about one
//! account per id, a live session receiving `ForgetSession`, and storage
//! mutation only after that session has stopped, but uses `oxidezap_session`
//! tasks and local-friendly primitives throughout.
//!
//! The handle is attached to the registry as a weak back-pointer, like the
//! native supervisor. The owner of the handle is the embedded service, so a
//! registry outliving that service cannot keep a stale lifecycle alive.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use oxidezap_core::AccountId;
use oxidezap_session::StoreRegistry;
use tokio::sync::{oneshot, watch};

#[cfg(test)]
use tokio::sync::mpsc;

use crate::session_bridge::{
    AccountDisposition, AccountExit, Action, CommandOutcome, SessionCommand, finish_forget,
};
use crate::state::StateHub;

use super::{AccountRegistry, LifecycleOutcome};

/// A future that can be held by this lifecycle on either target.
///
/// `oxidezap_session::spawn` is `Send` on a desktop and unrestricted in a
/// page. The trait object spells that out per target instead of inventing a
/// new bound: the native alias says `Send`, the web alias says nothing.
#[cfg(not(target_family = "wasm"))]
pub(crate) type LocalFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
#[cfg(target_family = "wasm")]
pub(crate) type LocalFuture<T> = Pin<Box<dyn Future<Output = T> + 'static>>;

#[cfg(not(target_family = "wasm"))]
pub(crate) type AllocateFn =
    dyn Fn(Arc<StoreRegistry>) -> LocalFuture<anyhow::Result<AccountId>> + Send + Sync;
#[cfg(target_family = "wasm")]
pub(crate) type AllocateFn = dyn Fn(Arc<StoreRegistry>) -> LocalFuture<anyhow::Result<AccountId>>;

#[cfg(not(target_family = "wasm"))]
pub(crate) type StartFn = dyn Fn(
        Arc<AccountRegistry>,
        AccountId,
        Arc<StoreRegistry>,
        Arc<EmbeddedLifecycle>,
    ) -> LocalFuture<()>
    + Send
    + Sync;
#[cfg(target_family = "wasm")]
pub(crate) type StartFn = dyn Fn(
    Arc<AccountRegistry>,
    AccountId,
    Arc<StoreRegistry>,
    Arc<EmbeddedLifecycle>,
) -> LocalFuture<()>;

#[cfg(not(target_family = "wasm"))]
pub(crate) type ForgetFn = dyn Fn(AccountId, Arc<StoreRegistry>, Arc<StateHub>, AccountDisposition) -> LocalFuture<AccountExit>
    + Send
    + Sync;
#[cfg(target_family = "wasm")]
pub(crate) type ForgetFn = dyn Fn(
    AccountId,
    Arc<StoreRegistry>,
    Arc<StateHub>,
    AccountDisposition,
) -> LocalFuture<AccountExit>;

/// The embedded account lifecycle attached to one [`AccountRegistry`].
pub struct EmbeddedLifecycle {
    registry: Arc<AccountRegistry>,
    stores: Arc<StoreRegistry>,
    allocate: Arc<AllocateFn>,
    start: Arc<StartFn>,
    forget: Arc<ForgetFn>,
    /// Ids whose command channel currently has a receiver in a run loop.
    ///
    /// Used the same way the native supervisor uses its own active set: a
    /// terminal account has no receiver to ask, and no storage mutation may
    /// run while the previous loop is still closing its session.
    active: watch::Sender<HashSet<AccountId>>,
    /// Serializes lifecycle decisions with run-loop rebuilds. A single
    /// browser thread does not make this unnecessary: the control request
    /// and a run loop both await stores, so they can interleave otherwise.
    gate: tokio::sync::Mutex<()>,
    /// Invalidates an already armed restart sleep when a new account starts
    /// or a terminal lifecycle request retires the id.
    generations: RwLock<HashMap<AccountId, u64>>,
}

impl EmbeddedLifecycle {
    /// Build the production handle and attach it to `registry`.
    ///
    /// A page has one thread, but the daemon API shares handles as `Arc`
    /// there as well: changing that would split the control plane from the
    /// session bridge for no gain. The lint is desktop-oriented.
    #[cfg_attr(target_family = "wasm", allow(clippy::arc_with_non_send_sync))]
    #[must_use]
    pub fn attach(registry: Arc<AccountRegistry>, stores: Arc<StoreRegistry>) -> Arc<Self> {
        let lifecycle = Arc::new(Self {
            registry: Arc::clone(&registry),
            stores: Arc::clone(&stores),
            allocate: Arc::new(default_allocate),
            start: Arc::new(default_start),
            forget: Arc::new(default_forget),
            active: watch::channel(HashSet::new()).0,
            gate: tokio::sync::Mutex::new(()),
            generations: RwLock::new(HashMap::new()),
        });
        registry.attach_embedded_lifecycle(&lifecycle);
        lifecycle
    }

    /// Allocate one new account through the shared store registry and start
    /// its runtime before answering.
    ///
    /// `StoreRegistry::create_account` takes its id from SQLite's
    /// `AUTOINCREMENT`, so two concurrent callers cannot be handed the same
    /// id and a removed id is never reissued.
    pub async fn create_account(self: &Arc<Self>) -> anyhow::Result<AccountId> {
        let id = (self.allocate)(Arc::clone(&self.stores)).await?;
        (self.start)(
            Arc::clone(&self.registry),
            id,
            Arc::clone(&self.stores),
            Arc::clone(self),
        )
        .await;
        Ok(id)
    }

    /// Reset one account, keeping its id.
    pub async fn reset_account(self: &Arc<Self>, id: AccountId) -> LifecycleOutcome {
        self.request_lifecycle(id, AccountDisposition::Reset).await
    }

    /// Retire one account for good.
    pub async fn remove_account(self: &Arc<Self>, id: AccountId) -> LifecycleOutcome {
        self.request_lifecycle(id, AccountDisposition::Remove).await
    }

    async fn request_lifecycle(
        self: &Arc<Self>,
        id: AccountId,
        disposition: AccountDisposition,
    ) -> LifecycleOutcome {
        let _gate = self.gate.lock().await;
        let Some(runtime) = self.registry.get(id) else {
            return LifecycleOutcome::NoAccount;
        };
        if self.registry.has_retirement(id) {
            return LifecycleOutcome::NoAccount;
        }
        // The first reason an account stops owns the outcome. A second,
        // identical ask is the operation already in flight; a different one
        // must not be reported as accepted when it will never run.
        if runtime.is_stopping() {
            return if runtime.lifecycle().disposition() == Some(disposition) {
                LifecycleOutcome::Accepted
            } else {
                LifecycleOutcome::Incomplete
            };
        }

        let (answered, accepted) = oneshot::channel();
        let sent = runtime
            .commands()
            .send(SessionCommand {
                action: Action::ForgetSession(disposition),
                reply: answered,
            })
            .await;
        if sent.is_ok() {
            match accepted.await {
                Ok(CommandOutcome::Accepted) => return LifecycleOutcome::Accepted,
                Ok(_) => return LifecycleOutcome::Incomplete,
                Err(_) => {}
            }
        }

        // No live session owns the channel. This is the account that logged
        // out, spent its recovery attempts, or is between an end and its
        // restart sleep. Finish the storage mutation here instead of
        // answering `NoSession`, after the old loop has stopped writing.
        if runtime.lifecycle().set_disposition(disposition)
            == crate::session_bridge::DispositionOutcome::Conflict
        {
            return LifecycleOutcome::Incomplete;
        }
        self.bump_generation(id);
        let mut active = self.active.subscribe();
        let was_running = active.borrow_and_update().contains(&id);
        let lifecycle = Arc::clone(self);
        let forget = Arc::clone(&self.forget);
        oxidezap_session::spawn(async move {
            while active.borrow_and_update().contains(&id) {
                if active.changed().await.is_err() {
                    break;
                }
            }
            let Some(runtime) = lifecycle.registry.get(id) else {
                return;
            };
            // A teardown that did not close its session is not permission to
            // delete storage from under it. This is the same gate the native
            // supervisor applies before its direct path.
            if was_running && !runtime.lifecycle().is_quiesced() {
                log::warn!(
                    "account {} did not quiesce; terminal cleanup deferred",
                    id.get()
                );
                return;
            }
            let _relink = lifecycle.registry.relink.lock().await;
            if lifecycle.registry.has_retirement(id) {
                return;
            }
            let Some(runtime) = lifecycle.registry.get(id) else {
                return;
            };
            let _media = runtime.lifecycle().drain_media().await;
            let exit = forget(
                id,
                Arc::clone(&lifecycle.stores),
                runtime.hub(),
                disposition,
            )
            .await;
            match exit {
                AccountExit::ResetCompleted => {
                    lifecycle.registry.remove(id);
                    lifecycle.restart_terminal(id).await;
                }
                AccountExit::RemoveCompleted => {
                    lifecycle.registry.remove(id);
                }
                _ => {
                    runtime.lifecycle().retry_failed_disposition(disposition);
                    lifecycle
                        .registry
                        .set_status(id, oxidezap_ipc::AccountStatus::Error);
                }
            }
        });
        LifecycleOutcome::Accepted
    }

    /// Start a fresh runtime after a terminal reset completed directly.
    ///
    /// The run loop that owned the old session has already ended, so nothing
    /// else can respawn this id. A generation taken before the gate is still
    /// current only when no newer request or startup has superseded it.
    async fn restart_terminal(self: &Arc<Self>, id: AccountId) {
        let generation = self.generation(id);
        let _gate = self.gate.lock().await;
        if generation != self.generation(id)
            || self.registry.get(id).is_some()
            || self.registry.has_retirement(id)
        {
            return;
        }
        (self.start)(
            Arc::clone(&self.registry),
            id,
            Arc::clone(&self.stores),
            Arc::clone(self),
        )
        .await;
    }

    /// Start a runtime through the production path.
    pub(crate) async fn start(self: &Arc<Self>, id: AccountId, stores: Arc<StoreRegistry>) {
        let _gate = self.gate.lock().await;
        self.bump_generation(id);
        (self.start)(Arc::clone(&self.registry), id, stores, Arc::clone(self)).await;
    }

    pub(crate) fn begin_run(&self, id: AccountId) {
        self.active.send_modify(|active| {
            active.insert(id);
        });
    }

    pub(crate) fn end_run(&self, id: AccountId) {
        self.active.send_modify(|active| {
            active.remove(&id);
        });
    }

    pub(crate) async fn rebuild_gate(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.gate.lock().await
    }

    pub(crate) fn generation(&self, id: AccountId) -> u64 {
        self.generations
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id)
            .copied()
            .unwrap_or(0)
    }

    fn bump_generation(&self, id: AccountId) {
        let mut generations = self
            .generations
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = generations.entry(id).or_insert(0);
        *entry = entry.wrapping_add(1);
    }

    /// Build a handle with injected storage work, for tests that must not
    /// open a real session or touch an account database.
    #[cfg(test)]
    #[cfg_attr(target_family = "wasm", allow(clippy::arc_with_non_send_sync))]
    pub(crate) fn with_hooks(
        registry: Arc<AccountRegistry>,
        stores: Arc<StoreRegistry>,
        allocate: Arc<AllocateFn>,
        start: Arc<StartFn>,
        forget: Arc<ForgetFn>,
    ) -> Arc<Self> {
        let lifecycle = Arc::new(Self {
            registry: Arc::clone(&registry),
            stores: Arc::clone(&stores),
            allocate,
            start,
            forget,
            active: watch::channel(HashSet::new()).0,
            gate: tokio::sync::Mutex::new(()),
            generations: RwLock::new(HashMap::new()),
        });
        registry.attach_embedded_lifecycle(&lifecycle);
        lifecycle
    }
}

fn default_allocate(stores: Arc<StoreRegistry>) -> LocalFuture<anyhow::Result<AccountId>> {
    Box::pin(async move { stores.create_account().await.map_err(anyhow::Error::from) })
}

fn default_start(
    registry: Arc<AccountRegistry>,
    id: AccountId,
    stores: Arc<StoreRegistry>,
    lifecycle: Arc<EmbeddedLifecycle>,
) -> LocalFuture<()> {
    Box::pin(async move {
        registry.start_embedded(id, stores, Some(lifecycle)).await;
    })
}

fn default_forget(
    id: AccountId,
    stores: Arc<StoreRegistry>,
    hub: Arc<StateHub>,
    disposition: AccountDisposition,
) -> LocalFuture<AccountExit> {
    Box::pin(async move { finish_forget(id, &stores, &hub, disposition).await })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_bridge::CommandOutcome;
    use crate::state::StateHub;

    #[cfg(target_family = "wasm")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[cfg(not(target_family = "wasm"))]
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[cfg(not(target_family = "wasm"))]
    static DB: AtomicUsize = AtomicUsize::new(0);

    #[cfg(not(target_family = "wasm"))]
    fn stores() -> Arc<StoreRegistry> {
        let n = DB.fetch_add(1, Ordering::Relaxed);
        Arc::new(StoreRegistry::new(format!(
            "file:embedded-lifecycle-{n}-{}?mode=memory&cache=shared",
            std::process::id()
        )))
    }

    #[cfg(target_family = "wasm")]
    fn stores() -> Arc<StoreRegistry> {
        Arc::new(StoreRegistry::new(oxidezap_session::resolve_database_path()))
    }

    fn synthetic(
        id: AccountId,
        commands: mpsc::Sender<SessionCommand>,
    ) -> Arc<crate::account::AccountRuntime> {
        Arc::new(crate::account::AccountRuntime::new_with_registry(
            id,
            StateHub::for_account(id),
            Arc::new(oxidezap_plugin_host::Plugins::nothing_loaded(Arc::new(
                |_| {},
            ))),
            commands,
            stores(),
        ))
    }

    fn accepting_bridge() -> (
        mpsc::Sender<SessionCommand>,
        mpsc::UnboundedReceiver<Action>,
    ) {
        let (tx, mut rx) = mpsc::channel(4);
        let (taken_tx, taken_rx) = mpsc::unbounded_channel();
        oxidezap_session::spawn(async move {
            if let Some(SessionCommand { action, reply }) = rx.recv().await {
                let _ = reply.send(CommandOutcome::Accepted);
                let _ = taken_tx.send(action);
            }
        });
        (tx, taken_rx)
    }

    fn no_forget() -> Arc<ForgetFn> {
        Arc::new(|_, _, _, disposition| {
            Box::pin(async move {
                match disposition {
                    AccountDisposition::Reset => AccountExit::ResetCompleted,
                    AccountDisposition::Remove => AccountExit::RemoveCompleted,
                }
            })
        })
    }

    #[cfg_attr(target_family = "wasm", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_family = "wasm"), tokio::test)]
    async fn create_allocates_distinct_ids_and_starts_each_once() {
        let registry = AccountRegistry::new();
        let shared_stores = stores();
        let started: Arc<RwLock<Vec<AccountId>>> = Arc::new(RwLock::new(Vec::new()));
        let seen = Arc::clone(&started);
        let start: Arc<StartFn> = Arc::new(move |registry, id, stores, _lifecycle| {
            let seen = Arc::clone(&seen);
            Box::pin(async move {
                let (commands, _receiver) = mpsc::channel(4);
                assert!(registry.insert(synthetic(id, commands)));
                seen.write()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(id);
                let _ = stores;
            })
        });
        let lifecycle = EmbeddedLifecycle::with_hooks(
            Arc::clone(&registry),
            Arc::clone(&shared_stores),
            Arc::new(|stores| {
                Box::pin(async move { stores.create_account().await.map_err(anyhow::Error::from) })
            }),
            start,
            no_forget(),
        );

        let first = lifecycle.create_account().await.unwrap();
        let second = lifecycle.create_account().await.unwrap();
        assert_ne!(first, second);
        assert_eq!(
            started
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .as_slice(),
            &[first, second]
        );
        assert_eq!(registry.snapshot().accounts.len(), 2);
        assert_eq!(shared_stores.accounts().await.unwrap().len(), 2);
    }

    #[cfg_attr(target_family = "wasm", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_family = "wasm"), tokio::test)]
    async fn live_reset_and_remove_reach_only_the_named_account() {
        let registry = AccountRegistry::new();
        let first = AccountId::new(2).unwrap();
        let second = AccountId::new(3).unwrap();
        let (commands_first, mut taken_first) = accepting_bridge();
        let (commands_second, mut taken_second) = accepting_bridge();
        let first_runtime = synthetic(first, commands_first);
        let second_runtime = synthetic(second, commands_second);
        assert!(registry.insert(Arc::clone(&first_runtime)));
        assert!(registry.insert(Arc::clone(&second_runtime)));

        let lifecycle = EmbeddedLifecycle::with_hooks(
            Arc::clone(&registry),
            stores(),
            Arc::new(|_| Box::pin(async { Err(anyhow::anyhow!("not used")) })),
            Arc::new(|_, _, _, _| Box::pin(async {})),
            no_forget(),
        );

        assert_eq!(
            lifecycle.reset_account(first).await,
            LifecycleOutcome::Accepted
        );
        assert!(matches!(
            taken_first.recv().await.unwrap(),
            Action::ForgetSession(AccountDisposition::Reset)
        ));
        // The session records the disposition when it handles the command.
        assert!(matches!(
            first_runtime
                .lifecycle()
                .set_disposition(AccountDisposition::Reset),
            crate::session_bridge::DispositionOutcome::Recorded
                | crate::session_bridge::DispositionOutcome::AlreadySame
        ));

        assert_eq!(
            lifecycle.remove_account(second).await,
            LifecycleOutcome::Accepted
        );
        assert!(matches!(
            taken_second.recv().await.unwrap(),
            Action::ForgetSession(AccountDisposition::Remove)
        ));
        assert!(matches!(
            second_runtime
                .lifecycle()
                .set_disposition(AccountDisposition::Remove),
            crate::session_bridge::DispositionOutcome::Recorded
                | crate::session_bridge::DispositionOutcome::AlreadySame
        ));

        // A different request against the account already resetting loses to
        // the first one and says so instead of claiming a second teardown.
        assert_eq!(
            lifecycle.remove_account(first).await,
            LifecycleOutcome::Incomplete
        );
        assert_eq!(registry.snapshot().accounts.len(), 2);
    }

    #[cfg_attr(target_family = "wasm", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_family = "wasm"), tokio::test)]
    async fn terminal_account_completes_without_touching_the_other_one() {
        let registry = AccountRegistry::new();
        let terminal = AccountId::new(4).unwrap();
        let live = AccountId::new(5).unwrap();
        let (commands, mut taken_live) = accepting_bridge();
        // The terminal account has no run loop, so its command receiver is
        // gone. The other account is live and must never see this request.
        assert!(registry.insert(synthetic(terminal, commands_for_drop())));
        assert!(registry.insert(synthetic(live, commands)));

        let forgotten: Arc<RwLock<Vec<(AccountId, AccountDisposition)>>> =
            Arc::new(RwLock::new(Vec::new()));
        let (done_tx, done_rx) = oneshot::channel();
        let done = Arc::new(RwLock::new(Some(done_tx)));
        let seen = Arc::clone(&forgotten);
        let signalled = Arc::clone(&done);
        let forget: Arc<ForgetFn> = Arc::new(move |id, _, _, disposition| {
            let seen = Arc::clone(&seen);
            let signalled = Arc::clone(&signalled);
            Box::pin(async move {
                seen.write()
                    .unwrap_or_else(|error| error.into_inner())
                    .push((id, disposition));
                if let Some(tx) = signalled
                    .write()
                    .unwrap_or_else(|error| error.into_inner())
                    .take()
                {
                    let _ = tx.send(());
                }
                match disposition {
                    AccountDisposition::Reset => AccountExit::ResetCompleted,
                    AccountDisposition::Remove => AccountExit::RemoveCompleted,
                }
            })
        });
        let lifecycle = EmbeddedLifecycle::with_hooks(
            Arc::clone(&registry),
            stores(),
            Arc::new(|_| Box::pin(async { Err(anyhow::anyhow!("not used")) })),
            Arc::new(|_, _, _, _| Box::pin(async {})),
            forget,
        );

        assert_eq!(
            lifecycle.remove_account(terminal).await,
            LifecycleOutcome::Accepted
        );
        done_rx.await.unwrap();
        assert_eq!(
            forgotten
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .as_slice(),
            &[(terminal, AccountDisposition::Remove)]
        );
        assert!(registry.get(terminal).is_none());
        assert!(registry.get(live).is_some());
        assert!(taken_live.try_recv().is_err());
    }

    #[cfg_attr(target_family = "wasm", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_family = "wasm"), tokio::test)]
    async fn concurrent_create_reset_remove_keep_effects_scoped_and_final_state_stable() {
        let registry = AccountRegistry::new();
        let shared_stores = stores();
        let first = AccountId::new(6).unwrap();
        let second = AccountId::new(7).unwrap();
        let first_old = synthetic(first, commands_for_drop());
        assert!(registry.insert(Arc::clone(&first_old)));
        assert!(registry.insert(synthetic(second, commands_for_drop())));

        let (starts_tx, mut starts_rx) = mpsc::unbounded_channel::<AccountId>();
        let (forgets_tx, mut forgets_rx) =
            mpsc::unbounded_channel::<(AccountId, AccountDisposition)>();
        let start: Arc<StartFn> = Arc::new(move |registry, id, _, _| {
            let starts = starts_tx.clone();
            Box::pin(async move {
                assert!(registry.insert(synthetic(id, commands_for_drop())));
                let _ = starts.send(id);
            })
        });
        let forget: Arc<ForgetFn> = Arc::new(move |id, _, _, disposition| {
            let forgets = forgets_tx.clone();
            Box::pin(async move {
                let _ = forgets.send((id, disposition));
                disposition.completed()
            })
        });
        let lifecycle = EmbeddedLifecycle::with_hooks(
            Arc::clone(&registry),
            Arc::clone(&shared_stores),
            Arc::new(|stores| {
                Box::pin(async move { stores.create_account().await.map_err(anyhow::Error::from) })
            }),
            start,
            forget,
        );

        let (created, reset, removed) = tokio::join!(
            lifecycle.create_account(),
            lifecycle.reset_account(first),
            lifecycle.remove_account(second),
        );
        let created = created.expect("a synthetic account was allocated");
        assert_eq!(reset, LifecycleOutcome::Accepted);
        assert_eq!(removed, LifecycleOutcome::Accepted);
        assert_ne!(created, first);
        assert_ne!(created, second);

        let mut forgets = Vec::new();
        for _ in 0..2 {
            forgets.push(forgets_rx.recv().await.expect("one forget per request"));
        }
        forgets.sort_by_key(|(id, _)| id.get());
        assert_eq!(
            forgets,
            vec![
                (first, AccountDisposition::Reset),
                (second, AccountDisposition::Remove)
            ]
        );

        let mut starts = Vec::new();
        for _ in 0..2 {
            starts.push(starts_rx.recv().await.expect("one start per surviving id"));
        }
        starts.sort_by_key(|id| id.get());
        let mut expected = vec![first, created];
        expected.sort_by_key(|id| id.get());
        assert_eq!(starts, expected);

        let mut ids: Vec<_> = registry
            .runtimes()
            .iter()
            .map(|runtime| runtime.id())
            .collect();
        ids.sort_by_key(|id| id.get());
        assert_eq!(ids, expected);
        let replacement = registry.get(first).expect("the reset id came back");
        assert!(
            !Arc::ptr_eq(&replacement, &first_old),
            "the reset inserted a fresh runtime, not the old one"
        );
        assert_eq!(replacement.lifecycle().disposition(), None);
        assert!(registry.get(second).is_none());
        let stored = shared_stores.accounts().await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, created);
    }

    #[cfg_attr(target_family = "wasm", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_family = "wasm"), tokio::test)]
    async fn reset_racing_remove_performs_one_forget_and_one_storage_completion() {
        let registry = AccountRegistry::new();
        let shared_stores = stores();
        let id = shared_stores
            .create_account()
            .await
            .expect("a synthetic account row");
        let account = synthetic(id, commands_for_drop());
        assert!(registry.insert(Arc::clone(&account)));
        let mut accounts = registry.subscribe();
        accounts.borrow_and_update();

        let calls = Arc::new(portable_atomic::AtomicUsize::new(0));
        let (completions_tx, mut completions_rx) =
            mpsc::unbounded_channel::<(AccountId, AccountDisposition, AccountExit)>();
        let forget_stores = Arc::clone(&shared_stores);
        let forget_calls = Arc::clone(&calls);
        let forget: Arc<ForgetFn> = Arc::new(move |id, _, _, disposition| {
            let stores = Arc::clone(&forget_stores);
            let calls = Arc::clone(&forget_calls);
            let completions = completions_tx.clone();
            Box::pin(async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                match disposition {
                    AccountDisposition::Reset => stores
                        .reset_account(id)
                        .await
                        .expect("real reset of the synthetic row"),
                    AccountDisposition::Remove => stores
                        .remove_account(id)
                        .await
                        .expect("real removal of the synthetic row"),
                }
                let exit = disposition.completed();
                let _ = completions.send((id, disposition, exit));
                exit
            })
        });
        let lifecycle = EmbeddedLifecycle::with_hooks(
            Arc::clone(&registry),
            Arc::clone(&shared_stores),
            Arc::new(|_| Box::pin(async { Err(anyhow::anyhow!("create is not used")) })),
            Arc::new(|_, _, _, _| Box::pin(async {})),
            forget,
        );

        let lifecycle_for_remove = Arc::clone(&lifecycle);
        let lifecycle_for_reset = Arc::clone(&lifecycle);
        let (removed, reset) = tokio::join!(
            async move { lifecycle_for_remove.remove_account(id).await },
            async move { lifecycle_for_reset.reset_account(id).await }
        );
        assert_eq!(removed, LifecycleOutcome::Accepted);
        assert_eq!(
            reset,
            LifecycleOutcome::Incomplete,
            "remove owns the disposition, so reset cannot claim a second teardown"
        );
        let (completed_id, disposition, exit) = completions_rx
            .recv()
            .await
            .expect("the real storage completion was signalled");
        assert_eq!(completed_id, id);
        assert_eq!(disposition, AccountDisposition::Remove);
        assert_eq!(exit, AccountExit::RemoveCompleted);
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the loser never ran another storage mutation"
        );
        assert_eq!(
            account.lifecycle().disposition(),
            Some(AccountDisposition::Remove)
        );

        assert!(registry.get(id).is_none());
        assert!(
            accounts.has_changed().unwrap(),
            "the terminal removal published an AccountsChanged snapshot"
        );
        assert!(accounts.borrow_and_update().accounts.is_empty());
        assert!(
            !shared_stores
                .accounts()
                .await
                .unwrap()
                .iter()
                .any(|stored| stored.id == id)
        );
    }

    fn commands_for_drop() -> mpsc::Sender<SessionCommand> {
        let (commands, _receiver) = mpsc::channel(4);
        commands
    }
}
