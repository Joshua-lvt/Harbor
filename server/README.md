# Harbor Relay (legado)

> **Backend legado.** O relay do Harbor agora vive em
> [`../harbor-cloud`](../harbor-cloud) (Cloudflare Worker + Durable Objects). Esta
> pasta é a fonte 1:1 de onde o Worker foi portado, mantida dormente como
> referência. Veja [`LEGACY.md`](LEGACY.md). Não inicie trabalho novo aqui.

Relay WebSocket privado que faz a ponte entre pareamento, presença e chat entre
**exatamente dois dispositivos pareados**. A ideia é rodar uma instância em
algum lugar que os dois dispositivos consigam alcançar (um VPS privado, um
servidor caseiro, ou um nó Tailscale) — **não** é uma plataforma pública.

Este servidor foi a primeira encarnação do backend do Harbor. A funcionalidade
matouza foi portada para um Cloudflare Worker em [`../harbor-cloud`](../harbor-cloud),
que oferece as mesmas rotas HTTP, o mesmo protocolo WebSocket e o mesmo
encaminhamento cego de criptografia ponta-a-ponta — só que rodando no edge,
sempre ativo e globalmente acessível. Este diretório fica no disco para que a
migração possa ser auditada contra ele e servir de fallback até o Worker ser
provado em produção com dois clientes reais.

## Rodar (dev)

```sh
python -m venv .venv
.venv\Scripts\activate          # Windows
pip install -e ".[dev]"
uvicorn app.main:app --host 0.0.0.0 --port 8000 --reload
```

## Testes

```sh
pytest
```

A suite `pytest` é deixada verde de propósito: ela funciona como a especificação
comportamental contra a qual o Worker (em `harbor-cloud`) foi validado.

## Rotas

| Método | Caminho | Body / Query | Retorna |
|---|---|---|---|
| `POST` | `/register` | `{device_id}` | `{pairing_code, device_secret}` |
| `POST` | `/pair` | `{device_id, device_secret, partner_code}` | `{partner_device_id, partner_name}` |
| `POST` | `/profile` | `{device_id, device_secret, display_name}` | `{ok}` |
| `GET`  | `/partner` | `?device_id=&secret=` | `{partner_device_id, partner_name, presence, last_seen}` |
| `GET`  | `/health` | | `{status: ok}` |
| `WS`   | `/ws` | `?device_id=&secret=` | envelopes de presença / typing / chat / ack / last_seen |

## Envelopes WebSocket (JSON)

- `{"type":"chat","id","text"}` → o relay ecoa `{"type":"chat",...,"from","ts"}`
  ao parceiro, ou guarda num buffer se ele estiver offline; o remetente recebe
  `{"type":"ack","id","delivered":bool}`.
- `{"type":"presence","state":"online|away"}` → encaminhado ao parceiro.
- `{"type":"typing","state":"start|stop"}` → encaminhado (transiente, não é
  armazenado).
- `{"type":"heartbeat"}` → atualiza `last_seen`/online.
- `{"type":"last_seen"}` → responde com presença atual do parceiro + `last_seen`.

## Produção / TLS

Rode atrás de um terminador TLS (recomendado Caddy com auto-cert) para que os
clientes usem `wss://`. O relay **precisa** rodar com `--workers 1` — o
`ConnectionManager` é em memória, por processo. Mensagens entregues são apagadas
na hora; as não entregues são varridas após `HARBOR_OFFLINE_MSG_TTL_DAYS` (7 por
padrão).

> **Criptografia:** este relay legado só encaminha texto cifrado opaco. A
> criptografia ponta-a-ponta é **no cliente** (libsodium `crypto_box_seal`,
> X25519 + XSalsa20-Poly1305) e foi construída depois deste relay — veja
> `../harbor-cloud/README.md` § Modelo de segurança. De qualquer forma, mantenha
> o relay privado e atrás de TLS.
