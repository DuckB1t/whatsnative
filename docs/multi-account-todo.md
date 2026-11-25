# Multi-account V2, TODO

Fonte de requisitos: `multi-account-plan-v2.md`, o documento de handoff mantido
em paralelo a este arquivo, atualizar os dois a cada marco. Ele não vive neste
repositório; este arquivo é o resumo canônico do estado da implementação.
A implementação usa **um `whatsapp.db` compartilhado**; `AccountId` é o `device.id`
positivo desse banco. Não reintroduzir `accounts.json`, UUIDs, um DB por conta ou
migração de arquivo.

**PR aberto (draft):** `https://github.com/oxidezap/client/pull/172` (branch
`feat/multi-account`). CI estava vermelho por um conflito de ordenação de
migrations com o PR #171 (mergeado em `main` depois que esta branch foi
criada); corrigido e re-empurrado, ver a entrada "CI do PR #172 corrigido"
abaixo para o diagnóstico completo.

## Estado atual

- [x] Corrigir `AccountId` para `i32` opaco, validado (`> 0`) e serializável.
- [x] Preservar `AccountId(1)` como slot legado da instalação existente.
- [x] Abrir o backend WhatsApp por `device_id` através de `StoreRegistry`.
- [x] Adicionar foreign keys `device_id -> device(id) ON DELETE CASCADE` ao
      `chat-store` para chats, mensagens, reações, contatos, receipts e mídia.
- [x] Testar isolamento A/B no mesmo arquivo: remoção das rows de A preserva
      integralmente as rows de B.
- [x] Atualizar fixtures para respeitar a tabela pai `device` e validar o
      comportamento com o store público upstream.
- [x] Validações concluídas: `fmt`, `core`, `chat-store`, `session` isolado e
      `check` do `session`.
- [x] Investigar a sensibilidade de scheduling no teste completo de `session`:
      a execução paralela apresentou uma falha transitória, mas a suíte completa
      passou com `--test-threads=1`, e o teste também passa isolado.
      A invariável de sessão meio-aberta foi mantida; nenhuma asserção foi
      enfraquecida.
- [x] **WR-1 upstream mergeado.** `https://github.com/oxidezap/whatsapp-rust/pull/1505`,
      merged como `3a08e84e8d7a462f6f87eb98d28fe1ce889a653a` em `main`. Publica em
      `SqliteStore` (= `whatsapp_rust_sqlite_storage::SqliteStore`, reexportado):
      `list_devices()`, `create_sibling_device()`, `reset_device(id)`,
      `remove_device(id)`. Alocação de id via `AUTOINCREMENT` +
      `last_insert_rowid()` na mesma transação (livre de corrida; testes
      upstream cobrem 16 criações concorrentes). `reset_device` apaga e
      recria a row `device` com o mesmo id e chaves novas, em uma única
      `BEGIN IMMEDIATE`; `remove_device` apaga a row e não a recria. Ambos
      chamam `purge_account_state`, que varre `ACCOUNT_SCOPED_TABLES`
      (lista com teste de cobertura de schema upstream).
      **Obrigação do chamador, documentada e deliberada:** nem `reset_device`
      nem `remove_device` invalidam um handle já aberto para aquele id, uma
      sessão ainda escrevendo pelo handle antigo pode repopular o que acabou
      de ser limpo. Quem chama (`StoreRegistry`, e por trás dele o daemon)
      tem que parar completamente a sessão/writer daquela conta antes de
      chamar reset/remove. Isso é política upstream, não faltando tratar.
- [x] Reapontar o client para a revisão upstream que contém WR-1
      (`3a08e84e8d7a462f6f87eb98d28fe1ce889a653a`, via `cargo update` nas
      cinco crates git juntas, nunca pinadas individualmente). Nenhum SQL
      raw contra a tabela privada `device` no client; `StoreRegistry` é o
      único ponto de conversão `AccountId <-> device_id`.
- [x] **Topologia de pool: uma pool compartilhada, não uma por conta.**
      `StoreRegistry` abre um único `SqliteStore` base e deriva o handle de
      cada conta via `share_for_device(id)`, em vez de abrir N pools
      independentes com `new_for_device`. Isso não é otimização: duas pools
      independentes apontando pro mesmo arquivo físico podem entrar no
      deadlock de upgrade leitura-depois-escrita que o próprio
      `SqliteStoreConfig::pool_size` documenta upstream, `busy_timeout` não
      resolve esse caso porque não é `SQLITE_BUSY` comum, é duas transações
      `DEFERRED` cada uma tentando promover pra writer ao mesmo tempo. Uma
      pool só faz duas contas serem o mesmo writer, então o deadlock não tem
      segunda parte pra acontecer com. Esse é o default que a seção 7 do
      plano pede ("começar compartilhado"), não o fallback "pool por device".
      Descoberto ao investigar uma falha real e intermitente
      (`"database is locked"`) num teste que escrevia em duas contas
      concorrentemente sobre o mesmo arquivo real, não era flakiness do
      teste, era o bug de arquitetura que o plano já avisava para evitar.
- [x] IPC v30: handshake com `ClientScope`, capacidades independentes e
      mensagens de control plane (`ControlHello`, listagem e snapshot de
      accounts).
- [x] Servidor resolve `Account { account }` no `AccountRegistry`, recusa
      account id desconhecido e impede requests de control em conexões de conta
      e requests de conta em conexões de control.
- [x] `CreateAccount`/`ResetAccount`/`RemoveAccount` respondem de verdade em
      `serve_control_client`: a primeira passa pelo `AccountSupervisor`
      (`create_and_spawn`, resposta `AccountCreated`) e as outras duas
      despacham `Action::ForgetSession(disposition)` para o `Commands` da conta
      alvo (resposta `Accepted`; id desconhecido responde `NoSession`). A
      *storage* por trás (`StoreRegistry::create_account/reset_account/
      remove_account`) já existia e está testada.
- [x] Preparação única do schema do `chat-store`: `StoreRegistry` guarda um
      `OnceCell` compartilhado, `AccountRuntime`/`WhatsAppClient` recebem o
      mesmo `StoreRegistry` e cada runtime abre apenas seu próprio writer com
      `ChatStore::new_prepared`. N runtimes iniciando ao mesmo tempo não disparam
      N migration runners contra o mesmo arquivo.
- [x] **`ForgetSession` ("clear data and pair again") já usa `reset_account`,
      não mais `wipe_local_state()`.** A chamada antiga apagava o arquivo
      `whatsapp.db` inteiro, o que derrubaria qualquer outra conta local que
      compartilhasse o arquivo, incompatível com multi-account por
      definição. A ordem de teardown que já existia (fechar sessão, esperar
      `close()`, juntar plugins, parar publisher, retirar approvals de
      plugin, só então mexer em storage) já cumpre a obrigação do WR-1 acima;
      só o último passo mudou de "apagar o arquivo" para "chamar
      `StoreRegistry::reset_account(id)`" (que também dispara o `ON DELETE
      CASCADE` da migration de chat-store, então as linhas do chat-store
      dessa conta somem de graça na mesma transação).
- [x] **Plugin state namespaced por conta.** `approvals`/`settings` de plugin
      vivem em `plugin-state/{account_id}/` no nativo
      (`crates/daemon/src/plugins/mod.rs::account_state_dir`) e em chaves de
      `localStorage` prefixadas por `AccountId` no web
      (`crates/plugin-host/src/store/web.rs`). Sem isso, uma segunda conta
      herdaria ou sobrescreveria as permissões da primeira. O catálogo de
      módulos `.wasm` continua global, como a seção 13.3 do plano pede. O
      staging de instalação também está separado: chaves `u-` são globais e
      chaves `a<id>-u-` pertencem a uma conta.
- [x] **Shutdown agora acorda todas as contas, não só uma.**
      `crate::shutdown::ShutdownSignal` trocou de `tokio::sync::Notify`
      (`notify_one` acorda exatamente um waiter, invisível com uma única
      tarefa de sessão, quebrado no dia em que N contas cada uma espera o
      próprio `shutdown.notified()`) para um `watch<bool>` interno, onde
      todo waiter observa o mesmo request. API pública
      (`request()`/`requested()`) inalterada; pré-requisito que o item 4 do
      bloqueio abaixo já sinalizava, agora resolvido antes do primeiro spawn
      dinâmico de segunda conta em produção.
- [x] **`Action::ForgetSession` agora carrega uma `AccountDisposition`**
      (`Reset` ou `Remove`) em vez do bool antigo. `RuntimeLifecycle` ganhou
      um slot "first-call-wins"; o teardown em `session_bridge::run()` faz
      match nela para chamar `StoreRegistry::reset_account` (mantém o id,
      respawna) ou `StoreRegistry::remove_account` (aposenta o id de vez),
      antes só a forma `Reset` existia. Os dois call sites atuais
      (self-service "clear data and pair again" e seu teste) continuam
      passando `Reset`, comportamento idêntico ao anterior.
- [x] **`AccountSupervisor` (nativo) implementado e ligado** em
      `crates/daemon/src/account/mod.rs`: `spawn(id)` monta hub/plugins/
      commands, registra e entrega a runtime ao reaper, que é o dono único do
      `JoinSet`, a tarefa de cada conta só roda a sessão e devolve um
      `AccountExit`, nunca chama `spawn` por dentro, o que elimina o deadlock
      entre `join_all()` e um respawn de `Reset` e o stall que um `spawn` de
      segunda conta sofria atrás de uma sessão longa. `spawn_with_hub(id, hub)`
      serve o bootstrap do `main.rs`, que no macOS constrói o `StateHub` na
      thread principal antes do runtime assíncrono (a tray). O reaper decide
      pelo `AccountExit`, não pelo pedido: `ResetCompleted` remove+respawna o
      mesmo id, `RemoveCompleted` só remove, `ResetIncomplete`/
      `RemoveIncomplete` deixam a runtime intacta, `SessionEnded` recupera com
      backoff e `SessionLoggedOut` deixa a conta para o usuário parear de novo.
      Gated `#[cfg(not(target_family = "wasm"))]`: `tokio::task::JoinSet`
      exige futures `Send`, e o host de plugin web é construído com closures
      `wasm-bindgen` deliberadamente `!Send`. `embedded.rs` já restaura um
      registry com todas as contas persistidas e compartilha o mesmo
      `CallCoordinator`. O lifecycle web não precisou de um supervisor
      `MaybeSend`: `account/lifecycle.rs` dá o mesmo contrato com tasks do
      `oxidezap_session` e um gate local, e `embedded.rs` é o dono do handle.
      Create/Reset/Remove já respondem na página (item 8).
- [x] **`DaemonMessage::AccountCreated { id, account }`** responde
      `ClientRequest::CreateAccount` no wire, nomeando o id que o daemon
      alocou, um cliente pode abrir uma conexão `Account`-scoped a ele
      assim que a mensagem chega, sem esperar `ListAccounts`/
      `AccountsChanged`. Comentários de doc das três requests de lifecycle
      não mencionam mais "recusado até WR-1" (WR-1 está mergeado).
- [x] **CI do PR #172 corrigido** (era vermelho antes deste commit):
      `account_device_cascade` e `message_stable_id` (que chegou via merge
      de `origin/main`, PR #171) compartilhavam o mesmo timestamp
      `2026-09-16-000000`. O tie-break do Diesel para duas migrations na
      mesma versão não é alfabético por nome de pasta, empiricamente,
      `account_device_cascade` rodava *depois* de `message_stable_id`,
      apesar de "account" < "message", e o `pull_request` trigger do GitHub
      testa o merge commit, não só o head da branch, então o conflito só
      apareceu no CI. Corrigido renomeando `account_device_cascade` para
      `2026-09-15-235959` (um segundo antes da meia-noite do dia anterior),
      e carregando a FK `device_id -> device(id)` que `account_device_cascade`
      adiciona para dentro do up.sql/down.sql de `message_stable_id` também
      (que reconstrói `messages` por cima e por isso é quem decide a forma
      final da tabela). Dois helpers de teste do chat-store também
      precisaram da tabela/row pai `device` que essa FK passou a exigir em
      todo caminho de migração (`queries.rs::plan_bound`,
      `storage_shape.rs::file_store`).

### Validação registrada

- `cargo fmt --all -- --check`, passou.
- `cargo test --workspace --lib -- --test-threads=1`, passou por completo
  (12 crates, ~900 testes) após a mudança de topologia de pool e o rewiring
  de `reset_account`/plugin-state.
- `cargo test --workspace --all-features --lib -- --test-threads=1`, passou
  por completo.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  sem warnings.
- `cargo check --workspace --all-targets`, passou.
- `cargo check -p oxidezap-daemon -p oxidezap-session --lib --target
  wasm32-unknown-unknown`, passou (mesmo alvo que o job `Test (web)` do CI
  compila antes de rodar no browser).
- `RUSTFLAGS="--cfg web_sys_unstable_apis" cargo clippy -p oxidezap-audio -p
  oxidezap-session --target wasm32-unknown-unknown -- -A clippy::all -A
  warnings -D clippy::disallowed_methods -D clippy::disallowed_types`,
  reproduz o "shared-view ban" do CI; passou.
- `cargo test -p oxidezap-session --lib -- --test-threads=1`, passou
  (169/169 antes das mudanças de plugin-state, que não tocam essa crate).
- Testes novos do `StoreRegistry`: `create_account_allocates_and_is_listed`,
  `ids_are_distinct_and_never_reused_after_removal`,
  `remove_retires_the_id_and_spares_other_accounts`,
  `reset_keeps_the_id_and_does_not_touch_other_accounts`, todos passam
  contra um arquivo temporário real (não o URL `cache=shared` em memória que
  o resto da suíte usa; ver nota de topologia de pool acima para o motivo).

### Validação desta sessão (fix de CI + `AccountSupervisor`)

- Após o merge de `origin/main` (traz o PR #171, `message_stable_id`) e a
  correção de ordenação de migrations: `cargo test -p oxidezap-chat-store
  --all-features`, passou por completo (44+ testes na suíte inline, todos os
  arquivos de teste de integração).
- `cargo test --workspace --all-features --no-fail-fast`, passou, com **uma
  única exceção pré-existente e não relacionada**:
  `whatsapp::tests::a_session_is_never_observed_half_open` (em
  `oxidezap-session`) é flaky sob execução paralela completa, confirmado
  isolado (`--exact`, passa sempre) e confirmado **também flaky em
  `origin/main` sem nenhuma mudança desta branch** (testado num worktree
  descartável). Não é uma regressão; não foi tocado.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
  --all-features -- -D warnings`, `cargo test --workspace --all-features
  --doc`, `cargo check --workspace --all-targets`, todos passaram limpos.
- `cargo test -p oxidezap-daemon -p oxidezap-session --lib --target
  wasm32-unknown-unknown --no-run`, falha no link ("--shared-memory is
  disallowed... not compiled with 'atomics'"), confirmado como limitação
  pré-existente deste sandbox local (reproduz igual num checkout limpo de
  `origin/main`); nenhum erro de compilação novo, só esse estágio de link.
  CI real (GitHub Actions) não roda esse comando exato da mesma forma,
  confiar no resultado do `Test (web)` do CI para essa parte.
- `RUSTFLAGS="--cfg web_sys_unstable_apis" cargo clippy -p oxidezap-audio -p
  oxidezap-session --target wasm32-unknown-unknown -- -A clippy::all -A
  warnings -D clippy::disallowed_methods -D clippy::disallowed_types`,
  passou.
- Diagnóstico completo do porquê o CI estava vermelho está no histórico de
  commit `fix(chat-store): make the cascade migration sort unambiguously
  before stable-id` na branch.

### Validação desta sessão (fiação do `AccountSupervisor`)

- Depois de ligar `AccountSupervisor` em `main.rs` (startup/shutdown) e nas
  três requests de lifecycle em `serve_control_client`: `cargo test -p
  oxidezap-daemon --all-features`, passou por completo (224+ testes na
  suíte inline, mais os arquivos de teste de integração), incluindo os dois
  testes novos (`supervisor_attaches_itself_to_its_registry_and_lets_it_go_when_dropped`
  em `account/mod.rs`; `create_account_is_refused_without_a_supervisor_attached`
  e `a_control_connection_resets_and_removes_a_named_account` em
  `server/tests.rs`), rodados 3x seguidas para descartar flakiness, todas
  estáveis.
- `cargo check -p oxidezap-daemon --lib --target wasm32-unknown-unknown`,
  passou: `serve_control_client` e seus dois novos helpers
  (`create_account`/`change_account`) compilam para wasm, onde
  `AccountSupervisor` não existe, o split de `cfg` fica dentro de
  `create_account`, nenhum listener precisou de um branch próprio.
- Matriz completa novamente: `cargo fmt --all -- --check`, `cargo clippy
  --workspace --all-targets --all-features -- -D warnings`, `cargo test
  --workspace --all-features --no-fail-fast` (mesma única exceção
  pré-existente de antes), `cargo check --workspace --all-targets`, `cargo
  test --workspace --all-features --doc`, os dois comandos de wasm do CI,
  todos passaram.

## Bloqueio arquitetural atual

~~A implementação de lifecycle de contas depende do WR-1 no
`whatsapp-rust`.~~ **Resolvido**. ~~Falta orquestração em tempo de execução
no daemon (`AccountSupervisor`).~~ **Resolvido**. ~~Falta ligar
`AccountSupervisor` em `main.rs` e nas três mutations do control plane.~~
**Resolvido nesta rodada**: `main.rs` agora usa `AccountSupervisor` no
startup (lista `StoreRegistry::accounts()`, spawna a conta legada pelo hub
pré-construído e as demais por `spawn`) e no shutdown (`join_all()` no lugar
do `JoinHandle` único; `shutdown::request()` no lugar do `Arc<Notify>`
local). `serve_control_client` responde de verdade `CreateAccount`
(`AccountSupervisor::create_and_spawn`, resposta `AccountCreated`) e
`ResetAccount`/`RemoveAccount` (despacha `Action::ForgetSession(disposition)`
para o `Commands` da conta alvo, resposta `Accepted`; id desconhecido
responde `NoSession`). `AccountRegistry` carrega um `Weak<AccountSupervisor>`
(setado por `AccountSupervisor::new`/`with_shutdown`) para que a conexão de
controle alcance o supervisor sem que nenhum listener precise passar um
segundo `Arc`, `Weak` porque o supervisor já segura um
`Arc<AccountRegistry>`, e um ponteiro forte de volta seria um ciclo que
nenhum dos dois solta.

**Não sobrou nenhuma peça de arquitetura pendente para o lifecycle dinâmico
de contas no daemon, nativo ou web.** O que falta agora é medição honesta:

1. **A GUI já fala v30 do jeito multi-conta nos dois alvos**: o
   `AccountWorkspace` troca só a conexão frontal, e o switcher com
   Add/Reset/Remove vive na aba Accounts das configurações. Na página o aviso
   fala do pré-requisito real, uma conexão de controle, não de um app de
   desktop que ali não existe.
2. **Web/embedded têm o próprio lifecycle**: `embedded.rs` ainda constrói
   runtimes à mão, porque o `AccountSupervisor` exige `Send` e o host de plugin
   da página não é, mas `account/lifecycle.rs` cobre o mesmo contrato. O
   `Service` é o dono do handle, o registry guarda só um `Weak`. Create aloca
   pelo `StoreRegistry` compartilhado e responde `AccountCreated` com a
   runtime registrada. Reset/Remove respondem `Accepted` como ack e terminam
   em background; a sessão viva recebe `ForgetSession`, e a conta terminal é
   limpa só depois que a task antiga parou. Não é uma clone do reaper nativo
   nem uma promessa de pool de threads, é o caminho que o navegador aguenta.
3. **Coordenação global (tray, calls) está pronta nos dois**: a tray já
   agrega. `crates/daemon/src/tray/accounts.rs` segue o `AccountRegistry` via
   `TrayHub::follow`, soma unread com `saturating_add` e usa `connected` se
   qualquer runtime estiver conectado. A Wave 4.3 acrescentou o
   `CallCoordinator` explícito em `call_coordinator.rs`, o marker de chamada na
   tray e o watch do dono em `TrayHub::follow`. O fence de hardware em
   `session_bridge/call_devices.rs` agora consulta o dono global antes de
   abrir preview, mantendo o scan local antigo como fallback de teste. O que
   resta é o gate de performance do item 10, não mais o registry.

**Decisão de produto do item 9/seção 20 (perguntada e respondida): o que
acontece com uma chamada ativa quando o usuário troca de conta.** Opção
escolhida: **A, finalizar a chamada.** Trocar de conta encerra qualquer
chamada ativa na conta que está sendo deixada; a UI deve avisar
explicitamente antes de agir ("trocar de conta vai encerrar sua chamada
 atual") e só prosseguir com confirmação do usuário, nunca finalizar a
chamada silenciosamente. A Wave 4.2 ligou essa resposta ao switcher nativo:
`switch_account` checa `call_state(cx).is_busy()`, mostra o aviso, e só
chama `hang_up` depois da confirmação explícita. Cancelar deixa a chamada e
a conta ativa intactas. A Wave 4.3 fechou a coordenação global: uma segunda
conta não pode tomar o slot enquanto a primeira tem uma chamada, e a tray
nomeia a conta dona. A mesma checagem serve a página, agora que o switcher
não está mais escondido nela. A confirmação continua sendo a única forma de
encerrar a chamada ao trocar de conta.

## Sequência de implementação

- [x] **1. Fundação**, `StoreRegistry` compartilhado (uma pool, não uma por
      conta), preparação única do DB, lifecycle completo (`create/reset/
      remove_account`) sobre WR-1, e testes de isolamento A/B no mesmo
      arquivo, incluindo reset/remove.
- [x] **2. Estado externo por conta**, namespace de media, avatar, wipe,
      plugin-state nativo e web por conta, e staging global de instalação
      prontos. O catálogo `.wasm` continua global de propósito.
- [x] **3. `AccountRuntime`**, encapsula `StateHub`, commands, plugin host e
      lifecycle por conta; o daemon/embedded/listeners compartilham um
      `AccountRegistry`, o bridge abre e reseta a sessão com o `AccountId`
      correto, e `AccountSupervisor` liga tudo isso em `main.rs`
      (spawn/respawn dinâmico, nativo). A coordenação global do item 9 e o
      lifecycle web/embedded do item 8 estão prontos; falta só o gate de
      performance do item 10.
- [x] **4. `AccountRegistry`**, N runtimes e snapshot/status isolados,
      `AccountSupervisor` como supervisor de tarefa por conta, e o
      `Weak<AccountSupervisor>` que deixa `serve_control_client` alcançá-lo
      sem uma segunda `Arc` em cada listener, tudo pronto e testado.
- [x] **5. IPC v30**, scopes `Control`/`Account`, handshake com `AccountId`,
      conexão de conta imutavelmente bound, listagem do registry, enforcement
      de requests, e as três mutações de lifecycle (`CreateAccount` ->
      `AccountCreated`, `ResetAccount`/`RemoveAccount` -> `Accepted`) todas
      respondendo de verdade em `serve_control_client`.
- [x] **6. GUI**, a **`ControlSession` já existe e está ligada**: a GUI abre
      uma segunda conexão `ClientScope::Control` (`Session::connect_control`,
      `attach::begin_control`) ao lado da de conta, e as requisições de
      processo (`SetLogLevel`, `ShowWindow`, `ReloadPlugins`, `InstallPlugin`,
      `RemovePlugin`, `ListInstalledPlugins`) foram movidas para ela, o plano
      de conta as recusava. A Wave 4.1 também decodifica `ControlHello`,
      `AccountsChanged` e `Accounts`, e expõe `list_accounts`,
      `create_account`, `reset_account` e `remove_account` no `SessionHandle`.
      A Wave 4.2 trocou o par `client`/`event_task` por um `AccountWorkspace`
      por conta ativa. O `control` sobrevive ao switch, a seleção é
      persistida, e um attach falho devolve a seleção anterior sem deixar uma
      UI meio trocada.
- [x] **7. UX**, switcher, Add, Reset, Remove, pairing por runtime e estado
      mínimo da conta ativa. A lista nativa mostra só id e status, sem nomes
      inventados. Add usa o id devolvido pelo control plane e deixa o pairing
      para o Hello daquela conta. Reset/Remove mostram "requested" e esperam
      `AccountsChanged`, nunca tratam `Accepted` como conclusão. Na web o
      aviso fala da conexão de controle, que é o pré-requisito verdadeiro.
- [x] **8. Web/embedded**, registry singleton, Web Lock no daemon, um DB OPFS,
      scopes de tab e restauração do conjunto inteiro, plugin-state por conta.
      A `ControlSession` cobre os quatro transports, e
      `account/lifecycle.rs` fecha o registry multi-conta embutido: Create
      aloca pelo mesmo `StoreRegistry`, Reset/Remove mandam `ForgetSession`
      para a sessão viva ou limpam storage depois da task antiga, e a
      confirmação de chamada ativa é a mesma da GUI nativa. Staging global
      `u-` de plugin agora funciona sem uma conta legacy viva. Persistência
      durável de execução em OPFS e testes de browser wasm continuam como
      gate de hardening, não como desculpa para dizer que isto está pronto.
- [x] **9. Global**, tray agregado, fence de preview entre runtimes,
      `CallCoordinator`, estado de chamada na tray e sinais cross-account
      prontos na Wave 4.3. A política determinística é o primeiro claim que
      vence, com o dono nomeado na recusa; um rename mantém o permit; uma
      liberação tardia não pode liberar a chamada de outra conta. Decisão de
      produto **já aplicada**: opção A, finalizar a chamada ativa ao trocar
      de conta, com aviso explícito antes de agir. O registry web usa a mesma
      checagem; o que resta é provar em navegador e medir, não inventar
      outra política.
- [x] **10. Hardening/performance**, matriz de races, testes de isolamento,
       stress de writer, benchmark 1/2/4/8 contas, web, docs e CI completo.
       **Gate sintético local fechado, não uma medição de contas conectadas.**
       Cada `WhatsAppClient` ainda cria um runtime Tokio multi-thread próprio
       mais uma thread que faz `block_on`, então 4 contas não são 4 sessões e
       sim 4 runtimes e seus pools. A Wave 4.4 acrescentou o harness com um
       `StoreRegistry` e N clients nunca iniciados. Esta rodada completou o
       que faltava: quatro races no supervisor nativo, duas no lifecycle
       embutido e duas no `CallCoordinator`, todas com barreiras ou oneshots em
       vez de sleeps. O `finish_terminal_with` agora guarda geração e ponteiro
       exato da runtime antes da limpeza, então uma conclusão terminal atrasada
       não apaga uma runtime mais nova com o mesmo id. O teste de writer usa um
       arquivo SQLite temporário real, 1/2/4/8 contas e writers concorrentes,
       e prova que ids iguais ficam isolados por conta. O modo `process` mediu
       1/2/4/8 contas com binário limpo, sem allocator de contagem, e passou a
       registrar startup frio, idle, shutdown, CPU, RSS/PSS, threads e FDs.
       Os números e os limites estão em `docs/daemon-memory.md`: threads e FDs
       crescem oito e quatro por conta adicional, mas CPU em ticks de 10 ms não
       sustenta custo por conta, e idle zero é apenas menor que um tick.
       History sync, reconexão, contas autenticadas, chamadas e mídia não têm
       substituto sintético honesto e continuam fora da reivindicação. Browser
       wasm roda no CI, não nesta máquina, e o primeiro sweep mostrou três
       assertions globais erradas nos testes de lifecycle web; elas agora
       comparam o delta de contas e os ids preservados. A "Definition of done"
       abaixo mantém os itens que exigem contas reais e artefato instalado como
       pendentes, sem fingir
       que um client nunca iniciado prova isso.

## Definition of done

- [ ] Duas ou mais contas conectadas simultaneamente no mesmo daemon e no
      mesmo `whatsapp.db`, com `device_id` distintos.
- [ ] Switch de conta troca somente a conexão frontend; nenhuma sessão WhatsApp
      é reconectada ou encerrada.
- [ ] Eventos, unread, mídia, plugins e permissões permanecem account-scoped.
- [x] Reset/remove de A não altera state, rows ou runtime de B/C, no storage
      e na runtime (testado no supervisor nativo e no lifecycle embutido, com
      fixtures sintéticas). Ids removidos nunca são reutilizados (testado, via
      `AUTOINCREMENT`), e o
      startup não recria um id removido: uma listagem vazia aloca pelo
      `create_account` em vez de fixar `AccountId(1)`.
- [ ] Operações assíncronas iniciadas em A não publicam em B após switch.
- [x] Falha de uma runtime não derruba daemon nem as demais: o reaper por
      `AccountExit`, com restart por backoff fora do loop (um timer por conta,
      guardado por geração), e uma conta que termina sozinha é recuperada sem
      bloquear as demais. Logout e recovery esgotado ficam observáveis como
      `Error` e ainda podem ser resetados/removidos pelo supervisor.
- [ ] Web, chamadas, tray, storage usage, migrations e benchmarks passam os
      gates do plano.
- [ ] `AGENTS.md`, `docs/architecture.md`, `docs/gotchas.md` e `docs/web.md`
      descrevem o invariante final.
