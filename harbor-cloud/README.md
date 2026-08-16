# harbor-cloud

O backend em **Cloudflare Workers + Durable Objects** do
[Harbor](../) — o relay privado de desktop para duas pessoas. É uma porta fiel
1:1 do [relay FastAPI legado](../server/) (`server/app/*`): as mesmas rotas HTTP,
o mesmo protocolo WebSocket, o mesmo encaminhamento cego de criptografia
ponta-a-ponta. Só o processo mudou — de um `uvicorn` que você mantinha rodando
na máquina de um operador, para um serviço de edge sempre ativo e globalmente
endereçável.

> O relay FastAPI legado em `../server/` permanece no disco como referência
> **LEGADA** (veja `../server/LEGACY.md`) até este Worker ser provado com dois
> clientes reais; então essas referências são removidas. A única coisa que muda
> do lado do cliente é a URL do relay: `ws://localhost:8000` (FastAPI) →
> `ws://localhost:8787` (`wrangler dev` local) →
> `wss://<worker>.workers.dev` (prod).

## Arquitetura

Duas classes de **Durable Object** com SQLite + um roteador **Worker**.

- **`HarborRegistry`** (`src/registry.ts`) — DO singleton
  (`idFromName("harbor-registry")`). Cuida das preocupações globais que um DO
  por par não consegue: lookup arbitrário de `pairing_code → device` e
  roteamento de `device → pair`. Mantém uma tabela SQLite `devices`. **Os
  segredos ficam só aqui.** Expõe as RPCs `register`, `pair`, `setProfile`,
  `getPartnerInfo`, `getMe`, `unpair`, e `verifyDevice` (auth do WS).
- **`HarborPair`** (`src/pair.ts`) — um DO por par (`idFromName(pair_key)`), o
  átomo de coordenação dos "exatamente-dois-dispositivos". Usa a **WebSocket
  Hibernation API** (sem cobrança de GB-s enquanto idle; clientes sobrevivem a
  evicção). Mantém em SQLite (`members`, `outbox`) o estado ao vivo e relevante
  a restart. Encaminha chat/presença/typing/atividade/`voice_signal`; faz buffer
  de chat offline no `outbox`; descarrega na reconexão com um `ack` tardio.
  Empurra presença `offline` depois de uma janela de carência de ~30s (não na
  hora — a única mudança de comportamento intencional em relação ao FastAPI).
- **Roteador Worker** (`src/index.ts`) — stateless: autentica via o Registry
  (mutações HTTP + upgrade do `/ws`) e roteia HTTP para RPC do Registry, e o
  socket `/ws` elevado para `HarborPair.fetch`. Não guarda segredos, não guarda
  estado por-conexão. CORS é permissivo (o FastAPI permitia tudo; o origin do
  WebView do cliente Tauri varia).

Arquivos: `src/protocol.ts` (unões do wire + `validateClientMessage` — a fonte
única da verdade dos formatos), `src/util.ts` (helpers de code/secret/pair-key
portados de `server/app/security.py` + `pairing.py`).

## Modelo de segurança

Inalterado em relação ao FastAPI. A criptografia E2E fica **no cliente**:
libsodium `crypto_box_seal` (X25519 + XSalsa20-Poly1305); o Worker/DOs só
roteiam metadados + chaves públicas + texto cifrado opaco em base64 `enc` e
**nunca descriptografam**. Chaves privadas nunca saem do cliente
(`identity.json`). O `device_secret` é dado por-dispositivo na SQLite do DO
Registry, não um secret de deploy — o `wrangler` **não precisa de secrets**. O
áudio fica P2P (`voice_signal` é só sinalização; sem SFU/relay/processamento).

## Desenvolver

```sh
npm install
npx wrangler dev          # Worker local em http://localhost:8787 (DOs em memória)
npm run test              # vitest watch  — ou `npm run test:run` para uma única rodada
node scripts/ws_smoke.mjs # harness manual de dois clientes WS contra o Worker local
```

O `wrangler dev` materializa os bindings de DO + as vars do `wrangler.jsonc`
em memória. `.wrangler/state/v3/do/...` é criado no disco para persistir entre
restarts em dev. `npm run cf-typegen` regenera o `worker-configuration.d.ts`
depois de editar o `wrangler.jsonc` (já commitado; rode de novo se mexer nos
bindings).

### Testes

A suite de vitest (`@cloudflare/vitest-pool-workers`) roda cada teste dentro de
um isolate do Miniflare com os bindings reais do `wrangler.jsonc` — os testes
exercitam o roteador Worker + os DOs reais em memória, sem deploy. O storage é
resetado por caso via `reset()` do `cloudflare:test`.

- `test/protocol.test.ts` — unit de `validateClientMessage()` (paths
  enc/plaintext, filtro de `data:image/`, tipos desconhecidos).
- `test/registry.test.ts` — paths HTTP via `SELF.fetch` contra o Registry real:
  `/register`, `/pair` (uso único, self-pair, secret errado, 404), `/profile`
  (set/clear de avatar+pubkey), `/me`, `/partner`, `/unpair`.
- `test/pair.test.ts` — integração WS Hibernation: handshake/auth, presença
  online/offline + a janela de carência (via `runDurableObjectAlarm`), encaminha
  chat + `ack` + buffer offline + flush na reconexão + late-ack, sinalização de
  voz, atividade/typing, frames malformados/oversized (socket sobrevive),
  conexão duplicada (4409), `last_seen`, push de unpair, heartbeat.

```sh
npm run test:run   # 3 arquivos, 62 testes — todos verdes
```

## Deploy

**Deploy é um passo final gated.** A entrega da implementação para em "pronto
para deploy"; `wrangler deploy` só roda depois da revisão de código+testes e de
um smoke com dois clientes. Localmente o backend já está provado (vitest + o
harness WS abaixo).

```sh
npx wrangler deploy                    # live em https://harbor-cloud.<conta>.workers.dev
npx wrangler deploy --name <name>      # ou um nome de Worker customizado
```

Depois do deploy, capture a URL `https://…workers.dev` e aponte o cliente pra
ela construindo o cliente com o relay de produção embutido:

```sh
cd ../client
VITE_RELAY_URL=wss://harbor-cloud.<conta>.workers.dev npm run tauri build
```

O `VITE_RELAY_URL` é lido em **um lugar só** (`DEFAULT_RELAY_URL` em
`client/src/lib/types.ts`) e semeado em `DEFAULT_SETTINGS.relay_url`; todo o
resto deriva. Instalações existentes num padrão anterior (`ws://localhost:8000`
ou `ws://localhost:8787`) migram automaticamente no próximo load; URLs
customizadas reais são preservadas. Produção sobre `wss://` é implícito (o
Worker serve TLS).

### Smoke de produção

Um harness E2E (`scripts/e2e_marco2.mjs`) sobe dois clientes contra o Worker
deployado, com criptografia real libsodium, presença e reconexão. Aponte-o para
prod com as env vars `HARBOR_SMOKE_HTTP` / `HARBOR_SMOKE_WS`.

## Vars (`wrangler.jsonc`)

| Var | Padrão | Finalidade |
|---|---|---|
| `HARBOR_OFFLINE_GRACE_MS` | `30000` | Janela de carência antes de um push offline (anti-flap). |
| `HARBOR_OFFLINE_MSG_TTL_DAYS` | `7` | Idade da linha do outbox em que a varredura TTL a descarta. |
| `HARBOR_MAX_FRAME_BYTES` | `262144` | Cap de frame WS inbound; oversized → `{type:"error"}`, socket mantido. |
