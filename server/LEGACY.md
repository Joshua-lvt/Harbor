# `server/` — relay FastAPI legado

> **⚠ Este é o backend LEGADO.** É mantido no disco apenas como referência
> dormente.
>
> O relay do Harbor foi migrado para **`harbor-cloud`** — um projeto em Cloudflare
> Worker + Durable Objects que é uma porta fiel 1:1 do código desta pasta (as
> mesmas rotas HTTP, o mesmo protocolo WebSocket, o mesmo encaminhamento cego de
> criptografia ponta-a-ponta). Veja:
> - `../harbor-cloud/README.md` — como rodar/deployar o backend novo.
>
> **Não inicie trabalho novo aqui.** Este diretório serviu de fonte para a qual
> o Worker foi portado; ele permanece para que a migração possa ser auditada e
> como fallback até o Worker ser provado com dois clientes reais. Quando essa
> prova chegar, as referências a `127.0.0.1:8000` / `uvicorn` no cliente e neste
> README são removidas e esta pasta pode ser apagada.
>
> **Nada foi renomeado aqui** — os caminhos de import e o CLI
> (`uvicorn app.main:app`) estão intactos, para um operador poder rodá-lo ad hoc
> se necessário. A suite `pytest` legada (`server/tests/`) é intencionalmente
> **deixada verde e intacta** como a especificação comportamental contra a qual a
> porta foi validada.

## Artefatos legados também mantidos no disco (não apagados)

- `testharbor.bat` — helper de dev top-level para o fluxo do relay FastAPI.
- `server.zip` — um snapshot arquivado do relay.
- `client/setup-client.ps1` — helper de bootstrap do cliente pré-Cloudflare.

Esses são notados como legado. O backend Cloudflare e o build do cliente
migrado não os usam, e podem ser removidos junto com esta pasta assim que o
Worker estiver provado em produção.
