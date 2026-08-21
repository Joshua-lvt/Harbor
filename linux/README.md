# Harbor Linux

Variante Linux nativa do Harbor. O frontend permanece wire-compatible com a versao Windows e usa o mesmo relay, pairing, eventos WebSocket e formato de mensagens E2E.

## Requisitos

- Linux x86_64
- Rust 1.77 ou mais recente
- Node.js 18 ou mais recente e npm
- GTK 3, WebKitGTK 4.1 e AppIndicator/StatusNotifier
- PipeWire ou PulseAudio para chamadas WebRTC
- GStreamer 1.24+ com `gstreamer`, `gstreamer-audio`, `gstreamer-video`, `gstreamer-app`, além dos plugins `good` e `bad` quando disponíveis
- `pipewire`/`pipewire-pulse` para captura; em Wayland, um desktop portal com suporte de captura PipeWire
- `ximagesrc` (plugin X) somente para o fallback explícito X11
- `xdg-utils` para lookup de icones e abertura de recursos do desktop
- Wayland e XWayland sao suportados; quando XWayland esta disponivel, o launcher prefere X11 para evitar falhas GBM do WebKitGTK em alguns compositores
- GNOME/Mutter ou KDE fornecem a fonte D-Bus preferida para idle
- Em Hyprland, `loginctl` e usado quando logind expoe `IdleHint`; sem uma API de idle disponivel, o app retorna idle zero em vez de inventar um valor

## Desenvolvimento

```sh
npm install
npm run test
npm run build
cd src-tauri && cargo check
cd ..
npm run tauri dev
```

O relay local pode ser executado em outro terminal:

```sh
cd ../harbor-cloud
npx wrangler dev
```

A URL padrao de desenvolvimento e `ws://localhost:8787`. Para producao, defina `VITE_RELAY_URL` antes do build, por exemplo:

```sh
VITE_RELAY_URL=wss://harbor-cloud.example.workers.dev npm run tauri build
```

## Pacotes

O script de pacote aceita a lista de bundles:

```sh
./scripts/package.sh deb
./scripts/package.sh appimage,deb
```

O pacote Debian foi validado localmente. O updater so gera artefatos assinados quando `TAURI_SIGNING_PRIVATE_KEY` estiver configurada no ambiente de release.

Para AppImage, o Tauri chama `linuxdeploy`, `linuxdeploy-plugin-gtk` e `linuxdeploy-plugin-gstreamer`. O script cria aliases em `.tools/` quando as ferramentas do repositorio estao presentes. A copia antiga de `linuxdeploy` pode falhar ao aplicar `strip` em bibliotecas modernas com secoes ELF `RELR`; nesse caso use uma versao atual do linuxdeploy e mantenha o mesmo nome de executavel no PATH.

### Ponte nativa de mídia

A primeira fatia nativa está disponível pelos comandos Tauri `media_capabilities`,
`media_start`, `media_stop`, `media_set_ptt` e `media_receive_signal`. Ela usa
GStreamer com filas limitadas e emite `media_state`, `media_audio` e
`media_video` para o WebView. O envelope de signaling continua sendo o mesmo
`v: 1` (`offer`/`answer`/`ice`) e não contém credenciais.

A engine WebRTC JavaScript continua sendo o fallback de produção enquanto
`native_webrtc` for `false`; a integração `webrtc-rs` será habilitada somente
após os testes de loopback e de peer JS. O WebView usa o mesmo transporte de
signaling com Relay + Broadcast privado por sala e solicita TURN de curta
validade; se a sessão de mídia não estiver disponível, permanece no Relay sem
credenciais embutidas. Em Wayland, a captura de tela exige a
sessão PipeWire selecionada pelo portal. X11 só é usado quando `DISPLAY` existe
ou quando `HARBOR_FORCE_X11=1` foi definido explicitamente. O AppImage deve
incluir os plugins GStreamer necessários, e a máquina alvo ainda precisa dos
serviços PipeWire/portal do desktop.

Para executar o binario release com a compatibilidade grafica padrao:

```sh
./scripts/run.sh
```

Em Hyprland, o launcher usa XWayland quando `DISPLAY` existe e desativa o renderer DMA-BUF do WebKitGTK. Para testar Wayland puro, use `HARBOR_FORCE_WAYLAND=1 ./scripts/run.sh`. Se o app for aberto diretamente pelo AppImage, exporte as mesmas variaveis manualmente.

## Wayland, atividade e PTT

- No Hyprland, a janela ativa e consultada por `hyprctl activewindow -j` e o PID e resolvido via `/proc/<pid>/exe`.
- X11/XWayland e usado somente como fallback para compositores que nao oferecem o caminho Hyprland.
- A atividade envia apenas o nome do processo, nunca titulo de janela ou conteudo de tela.
- Com a janela Harbor focada, PTT usa eventos de teclado Wayland/WebView e a tecla padrao e Left Alt.
- Em segundo plano, o polling nativo de estado de tecla depende de X11/XWayland. Um compositor Wayland sem portal/global shortcut nao oferece um caminho universal para capturar uma tecla global; nesse caso o microfone permanece mutado em segundo plano e a limitacao e deliberada.

## Funcionalidades

A variante Linux inclui pairing ativo e passivo, identidade e chaves X25519, chat E2E, presenca online/away/offline, typing, sincronizacao e buffer offline, deteccao de atividade, historico de atividade, icones, notificacoes, quick notifications, perfil/avatar, personalizacao, widget, close-to-tray, autostart, chamadas WebRTC e unpair.

A comunicacao usa os mesmos envelopes `chat`, `ack`, `typing`, `presence`, `last_seen`, `activity`, `activity_icon`, `profile_update`, `voice_signal` e `unpaired` do cliente Windows. Nao existe pairing ou relay exclusivo do Linux.

## Validacao manual

1. Inicie o relay local.
2. Abra duas instancias/clientes e pareie pelos codigos.
3. Verifique chat E2E nos dois sentidos, presence, reconnect e mensagens offline.
4. Verifique Chrome/Discord/jogos na atividade e o historico.
5. Conceda acesso ao microfone e valide audio nos dois sentidos, PTT e reconnect.
6. Valide notificacoes, tray, widget, autostart, close-to-tray e unpair.
7. Para compatibilidade cruzada, repita o fluxo com um cliente Windows: Linux ativo com Windows passivo e o inverso.
