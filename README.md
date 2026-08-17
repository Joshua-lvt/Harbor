# Harbor

**Seu porto seguro digital.** Harbor é um aplicativo de desktop para casais — uma
sensação tranquila e constante de presença entre duas pessoas em duas máquinas
Windows. Pareie uma vez com um código e o Harbor mantém vocês próximos: presença
ao vivo, um chat privado só de vocês dois, uma chamada de voz sempre ativa, um
widget pequeno que flutua sobre o que quer que você esteja fazendo, notificações
nativas e consciência de atividade em tempo real. Sem contas, sem e-mail, sem
números de telefone, sem servidores públicos. Só vocês dois.

> *Mesmo longe, você ainda está por perto.*

---

## Como é usar

Você abre o Harbor e a primeira coisa não é um chat — é **o seu parceiro**. Uma
Home de duas colunas mostra se ele está online, ausente ou offline, o que ele
está fazendo agora, há quanto tempo vocês estão conectados e o status da chamada
de voz que ficou abata o tempo todo, silenciosa. Aí você fecha a janela. O Harbor
encolhe para um ícone na bandeja do sistema, a presença continua ativa, e um
widget sempre-acima-de-tudo mantém ele no canto da sua tela enquanto você
trabalha. Nada mais compete por atenção. Não é rede social; é o oposto.

## Recursos

- **Presença ao vivo** — online / ausente / offline guiada pela ociosidade real
  de todo o sistema (`GetLastInputInfo` do Windows), não apenas se a janela do
  Harbor está em foco. "Ausente" significa que a máquina está realmente parada.
- **Chat privado de duas pessoas** — mensagens são criptografadas ponta-a-ponta
  no cliente com libsodium (`crypto_box_seal`, X25519 + XSalsa20-Poly1305). O
  relay só encaminha texto cifrado opaco e nunca descriptografa. Imagens,
  confirmações de entrega e buffer offline (as mensagens chegam quando o outro
  lado reconecta).
- **Chamada de voz sempre ativa** — um canal de voz WebRTC P2P que abre
  automaticamente assim que os dois lados estão pareados e se reconecta sozinho
  depois de quedas. O áudio é ponto-a-ponto; o relay carrega só a sinalização.
  Não existe "botão de chamar" porque não precisa.
- **O widget** — um card pequeno, arrastável, sempre-acima-de-tudo que flutua sobre
  as suas outras janelas: avatar do parceiro, ponto de presença e o que ele está
  fazendo. Feche a janela principal e a presença continua. É o ponto inteiro.
- **Consciência de atividade** — veja no que seu parceiro está focado (e ele vê
  você), com ícones reais puxados dos próprios aplicativos. Compartilhe ou mantenha
  só pra você; um histórico cronológico de atividade deixa você revisar o dia.
- **Notificações nativas + um overlay** — quando a janela principal está em segundo
  plano, um overlay discreto sempre-acima-de-tudo traz quick-messages com um gesto
  "Responder" / "Abrir chat", pra responder sem perder o contexto.
- **Perfis & avatares** — seu nome e avatar, os dele, sincronizados ao vivo pelo
  WebSocket quando qualquer um dos dois muda.
- **Modo escuro** — um toque, ou segue o sistema.
- **Iniciar com o Windows** — o Harbor volta quando a máquina volta.
- **Atualizações obrigatórias e assinadas** — nenhuma versão antiga pode continuar
  rodando quando existe uma mais nova. Disparada por `git tag vX.Y.Z`, a release é
  construída e assinada criptograficamente; o app verifica ao abrir, bloqueia até
  atualizar, e nunca cai silenciosamente pra "atualizado" quando a rede ou o
  servidor cai. Sua identidade, pareamento, chaves e histórico de mensagens
  sobrevivem intactos a uma atualização (vivem em `%APPDATA%`, não na pasta de
  instalação).

## Estrutura do repositório

| Diretório | O que é |
|---|---|
| `client/` | O app de desktop Tauri v2 — React + TypeScript + Tailwind v4, com núcleo em Rust. Multi-janela (principal + widget + overlay), bandeja do sistema, notificações nativas, autostart, cripto E2E, o portão de atualização. |
| `harbor-cloud/` | **O backend**: um relay em Cloudflare Worker + Durable Objects que media pareamento, presença e chat entre exatamente dois dispositivos pareados. Sempre ativo e globalmente acessível. |
| `server/` | Relat FastAPI **LEGADO** — a fonte 1:1 de onde o Worker foi portado. Mantido só como referência dormente; o cliente não disca pra ele por padrão anymore. |

## Instalação

Instaladores Windows pré-construídos são publicados como [**GitHub Releases**](https://github.com/Joshua-lvt/Harbor/releases). Baixe o `Harbor_x.x.x_x64-setup.exe`,
rode, e abra o Harbor. Esse é o caminho oficial — esses instaladores são assinados
e são de onde o auto-updater do app puxa.

## Construir do código-fonte

O Harbor é feito para **Windows 11** e constrói com a toolchain Tauri v2.

Pré-requisitos (uma vez):

- **Node** 20+
- **Toolchain Rust** (`stable-msvc`) + **MSVC "Desktop development with C++" Build
  Tools** — via <https://rustup.rs> e o instalador do Visual Studio Build Tools
- Para o backend: uma conta Cloudflare + `wrangler` (`npx wrangler login` uma vez)

```sh
# Backend relay (Cloudflare Worker local)
cd harbor-cloud
npm install
npx wrangler dev            # http://localhost:8787

# App de desktop
cd client
npm install
npm run tauri dev           # disca no relay padrão
```

Um instalador assinado pra distribuição é produzido com:

```sh
cd client
VITE_RELAY_URL=wss://<seu>.workers.dev npm run tauri build
# -> client/src-tauri/target/release/bundle/nsis/Harbor_x.x.x_x64-setup.exe (+ .sig)
```

O `VITE_RELAY_URL` é lido num lugar só (`client/src/lib/types.ts`) e semeado nas
settings padrão; instalações existentes migram automaticamente pra um novo padrão
e preservam qualquer URL customizada que você setou.

## Arquitetura, em breve

- **Dois dispositivos, um código.** Sem contas. Um dispositivo registra e recebe um
  código de uso único `HARBOR-XXXX-XXXX`; o outro pareia com ele. O vínculo é
  exatamente dois, sempre.
- **E2E fica no cliente.** Chaves privadas nunca saem do dispositivo
  (`identity.json` em `%APPDATA%`). O relay/Worker só roteia chaves públicas,
  metadados e texto cifrado `enc` opaco — ele não consegue ler suas mensagens nem
  ouvir sua chamada.
- **O relay é no edge.** Um Cloudflare Worker, router stateless + dois Durable
  Objects com SQLite (`HarborRegistry` pra lookups globais de pareamento,
  `HarborPair` pra coordenação dos exatos-dois). Ele encaminha
  chat/presença/typing/atividade/voz, faz buffer de chat offline, e manda
  presença offline só depois de uma janela de carência (sem flap).
- **Atualizações fecham o loop.** O instalador assinado + seu sidecar de hash
  (`latest.json`) são assets da release; o app valida a assinatura contra uma
  chave pública embutida no binário antes de instalar qualquer coisa nova.

Veja [`harbor-cloud/README.md`](harbor-cloud/README.md) pro backend e
[`server/README.md`](server/README.md) pro relay legado.

## Status do projeto

O Harbor é um projeto pessoal — um app real, funcional, de propósito único por
design. Não é uma plataforma, não é multi-usuário, e não tem interesse em crescer.
Duas pessoas. Um porto seguro.
