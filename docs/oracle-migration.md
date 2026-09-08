# Migração do Harbor Server para Oracle — auditoria e plano

## Estado e regra de segurança

**Estado em 2026-09-08T03:44Z — CORTE EXECUTADO:** K11 parado (quiescência
03:43Z, boot desativado reversível, nada apagado); snapshot final copiado
hash-a-hash e instalado na Oracle (0600/0700, chave confere com cert offline);
Oracle única autoridade desde 03:44:29Z servindo o pin de produção `b9846…`
(smoke externo verde). Clientes ainda apontam ao K11: ondas de migração
pendentes. K11 parado é o rollback; Fase 18 (7 dias) corre até ~2026-09-15.
Em 2026-09-08 o operador ordenou ainda: remoção total de Nebula/Tailnet/
Tailscale do produto e K11 como backup frio apenas — executado (único caminho
restante: direto/STUN/TURN/relay via Oracle).

Este documento é um plano, não um registro de implantação. Nenhuma configuração
OCI, regra de rede, unidade systemd, cópia de estado ou entrada DNS descrita abaixo
deve ser tratada como já aplicada.

Regras invariantes:

- O K11 permanece funcional e é a única autoridade gravável até o corte.
- Nunca haverá escrita ativa simultânea no K11 e na Oracle. Uma cópia de estado
  não autoriza iniciar dois servidores graváveis.
- REMOVIDO POR DECISÃO DO OPERADOR em 2026-09-08 (pós-corte): Tailscale, Tailnet
  e Nebula foram eliminados do produto (`network/`, `packaging/nebula/`,
  `docs/network-*.md`, membro `harbor-network` do workspace; arquivos Tailnet
  já haviam sido deletados antes). Sem fallback VPN a partir daqui: o único
  caminho é direto/STUN/TURN/relay via Oracle. K11 fica parado como backup frio.
- Não se altera nem se registra conteúdo de chave privada. Checksums de arquivos
  sensíveis podem ser comparados; seus bytes nunca devem aparecer em logs,
  tickets, terminal compartilhado ou neste documento.
- Falha em qualquer gate interrompe a fase. O rollback indicado ocorre antes de
  prosseguir.

## Inventário observado

### Oracle

| Item | Fato observado |
| --- | --- |
| SSH público | `<oracle-ipv4>:22` alcançável |
| Sistema | Oracle Linux 9.8, `x86_64` |
| Rede do guest | `ens3` com `10.0.0.7/24` |
| IPv6 | somente link-local; nenhum IPv6 global |
| Mapeamento público | o mapeamento para `<oracle-ipv4>` funciona para SSH |
| Memória | 498 MiB de RAM e 498 MiB de swap |
| Disco raiz | 30 GiB, aproximadamente 25 GiB livres |
| Init | systemd 252 |
| SELinux | enforcing |
| Tempo | `chronyd` ativo |
| Firewall do guest | `firewalld` ativo; zona `public` permite somente `ssh` e `dhcpv6-client` |
| SSH | autenticação por senha desabilitada; chave pública habilitada; login de root por chave atualmente `without-password` |
| Harbor | nenhum serviço Harbor e nenhum listener Harbor |
| Exposição medida | TCP `9091` e `3478` externamente fechadas/filtradas |
| OCI agent | aproximadamente 70 MiB de RAM |

O estado de NSG, security list e reserva do IP público na OCI permanece
**desconhecido**. Esses fatos não podem ser inferidos de dentro do guest. O SSH
funcionar comprova apenas o caminho público atual para a porta 22; não comprova
reserva do IP nem regras para qualquer outra porta.

### K11

A raiz de estado autoritativa é `$HOME/harbor/state`. Os hashes abaixo são valores
observados abreviados; devem ser reconfirmados em formato SHA-256 completo no
procedimento de cópia, sem imprimir o conteúdo da chave.

O host é um LG K11+ `armv7l`, executando Termux sobre Android 7.1.2. Sua LAN usa
DHCP dinâmico (`192.168.1.7` observado desde 2026-09-05; antes
`192.168.1.6`) e o SSH do Termux escuta em `8022` somente na LAN. O IPv4 público
fica atrás de CGNAT e não entrega `9091`; o caminho público atual é o IPv6 global
`2804:d59:8777:ad00:3a30:f9ff:fe3e:de81`. Esses fatos operacionais vêm do
runbook do K11 e devem ser revalidados antes de depender deles para rollback,
porque lease, prefixo e condições da operadora podem mudar.

| Artefato | Modo | Tamanho | SHA-256 observado |
| --- | ---: | ---: | --- |
| `tls/cert.pem` | `0600` | 518 bytes | `b040…` |
| `tls/key.pem` | `0600` | 241 bytes | `fb185…` |
| `state/control-state-v1.json` | `0600` | 8106 bytes | `6e780…` |
| binário em execução | — | — | `1392…` |

O certificado tem CN `harbor-server`, validade de 1975 a 4096 e fingerprint
SHA-256 `b9846…`. A chave corresponde ao certificado. O fingerprint público
completo atualmente duplicado no repositório é
`b9846aed2e97bd741ae5a2a3de9ab37c1831d2372ca67f26f538bd279dd7271f`.
TCP e UDP `9091` estão ativos no K11. O K11 deve permanecer intocado como
autoridade até o corte, salvo a janela de manutenção explicitamente aprovada para
o snapshot final.

## Dependências verificadas no repositório

Valores de documentação e fixtures genéricas de teste não são endpoints de
produção. A coluna "classe" faz essa separação explicitamente.

| Área | Arquivo/conceito | Comportamento verificado | Classe e ação de migração |
| --- | --- | --- | --- |
| Default desktop | `native/HarborFacade.cpp:42-55`, `:870-885`; `qml/Main.qml:59-68,197-212` | Endpoint padrão é `[2804:d59:8777:ad00:3a30:f9ff:fe3e:de81]:9091`; contém o pin completo; após `server.config`, envia `server.configure` uma vez somente se não configurado. | Produção. Trocar pelo IP Oracle apenas com o mecanismo de migração para instalações existentes. |
| Default mobile | `Harbor-Mobile/qml/Host/HarborMobileHost.qml:121-126,179-195` | Duplica endpoint e pin; configura somente quando `configured !== true`, sem sobrescrever pin existente. | Produção. Atualizar em conjunto com desktop. |
| Contrato mobile | `Harbor-Mobile/tests/tst_MobileSettingsNoServer.qml:50-60` | Verifica literalmente endpoint e pin embutidos no host mobile. | Teste de contrato de um valor de produção; atualizar junto com o default. |
| Pin duplicado | `native/HarborFacade.cpp`, `Harbor-Mobile/qml/Host/HarborMobileHost.qml`, `Harbor-Mobile/README.md`, `docs/k11-runbook.md`, `server/k11/find-k11.sh` | O mesmo fingerprint `b9846…` aparece em C++, QML, documentação e script. | Produção/documentação operacional. Inventariar todas as cópias; não rotacionar certificado durante esta migração. |
| IPv6 atual | mesmos defaults e `docs/k11-runbook.md:176-191` | Literal global do K11 é o endpoint público distribuído. | Produção. Substituir pelo IP Oracle após os gates. |
| Placeholder LAN | `qml/i18n/en.js:685-693`, `qml/i18n/pt_BR.js:685-693` | Exibe `192.168.1.6:9091`, endereço antigo do K11. | Texto de UI obsoleto, não configuração ativa; corrigir separadamente para não induzir operação errada. |
| Configuração IPC | `core/harbor-core/src/app/mod.rs:3136-3188`; `native/HarborFacade.cpp:853-860,1118-1135,1174-1189` | `server.config` lê o pin persistido; `server.configure` valida endereço e fingerprint de 64 hex e grava; a facade atualiza estado e peers. | Produção. A gravação não migra conexões já abertas, chamadas nem URLs TURN. |
| Estado do cliente | `core/harbor-core/src/pairing.rs:20-59`; `core/harbor-core/src/storage.rs:29-65,74-117` | `server-pin-v1.json` guarda versão, endereço e fingerprint; diretório `0700`, escrita atômica `0600`; `HARBOR_STATE_DIR` tem precedência. | Persistente por cliente. Migrar apenas o endpoint antigo exato; preservar configurações customizadas. |
| Sessões do cliente | `core/harbor-core/src/server.rs:176-209`; `core/harbor-core/src/app/signaling.rs:867-890,917-941`; `core/harbor-core/src/app/mod.rs:3864-3907` | Cliente usa uma conexão TLS por instância; operações e URLs STUN/TURN recarregam o pin estático. | Produção. Exige reconexão e atualização coerente das dependências após mudança. |
| Resolução/candidates | `core/harbor-core/src/server.rs:110-139`; `app/mobile_link.rs:208-226,273-297`; `app/mod.rs:784-807` | Dial tenta o endpoint configurado; mobile descobre endpoint reflexivo, prioriza-o nos invites e atualiza candidates em mudanças de listener/perna. | Produção. Revalidar troca de rede e ausência de IPv6 Oracle. |
| Relay no cliente | `core/harbor-core/src/app/relay_client.rs`, `relay_crypto.rs`, `app/mod.rs`, `app/mobile_link.rs` | O fallback usa sessões relay, frames E2E cifrados e integração com as pernas bearer; há fluxo de arquivo no código em evolução. | Produção em evolução. Fixar escopo suportado após revisão do código e reconciliar documentação contraditória antes do gate. |
| Scripts K11 | `server/k11/run-server.sh`, `supervise.sh`, `boot-harbor` | Início idempotente, bind `[::]:9091`, supervisão TCP+UDP, restart e rotação de logs via Termux:Boot. | Produção K11. Não reutilizar como serviço Oracle; manter intactos para rollback. |
| Descoberta K11 | `server/k11/find-k11.sh` | Descobre LAN por SSH `8022` e pin TLS em `9091`; pin de produção é default. | Ferramenta operacional K11. Preservar; restrita ao laboratório K11 após o corte. |
| DDNS | `server/k11/ddns-update.sh` | Atualizador independente de provedor, primariamente AAAA; usa `HARBOR_DDNS_URL`, `@AAA@`, `@A@` opcional e não loga token/endereço. | Produção K11. Não reutilizar o atualizador para a Oracle. |
| Smoke | `server/k11/smoke.sh` | Verifica pin TLS, nonce STUN-lite e resposta TURN `401`; por padrão usa localhost e extrai fingerprint do log. | Ferramenta operacional reutilizável somente após parametrização/validação em Oracle. Não prova relay externo. |
| Servidor/bind | `server/harbor-server/src/main.rs:15-40,57-84` | `HARBOR_SERVER_STATE_DIR`; default por XDG/home; bind default `127.0.0.1:9091`; cert/key opcionais; TCP TLS e UDP STUN/TURN compartilham endereço e porta. | Produção. Unidade Oracle deve fixar diretório e bind; não depender dos defaults. |
| Estado servidor | `server/harbor-server/src/store.rs:1-93`; `server/harbor-server/src/transport.rs:498-571` | `control-state-v1.json` persiste identidades/relações por escrita atômica; `tls/cert.pem` e `tls/key.pem` são carregados ou gerados. Códigos, presença e sessões são transitórios. | Persistente. Copiar snapshot e identidade TLS exatos, com modos privados. |
| TURN e mídia | `server/harbor-server/src/stun.rs`, `turn.rs`; `media/cmd/harbor-media/main.go`; `core/harbor-core/src/app/signaling.rs` | Servidor demultiplexa STUN/TURN no UDP `9091`; credenciais/URLs vêm do controle. A prova de chamada física via TURN permanece necessária. | Produção em evolução. Bloqueada pelo anúncio/roteamento público e pelo gate físico. |
| CI | `.github/workflows/build.yml` | Compila/testa Linux, QML, CTest, Windows e Android; não há lint Markdown específico. | CI/empacotamento. Mudanças futuras de defaults devem passar todos os jobs aplicáveis. |
| Empacotamento | `CMakeLists.txt`; `packaging/windows/harbor.nsi`; manifest/gradle Android | Instala binários/desktop/icons e integra plataformas; este Markdown não é empacotado. | Removidos em 2026-09-08 por decisão do operador: `packaging/nebula/`, restos Tailnet/Tailscale (já deletados antes). Validar instaladores Windows e Android físicos. |
| Runbook/documentação de produto | `Harbor-Mobile/README.md`, `docs/k11-runbook.md`, `README.md` | Os dois primeiros registram endpoint/pin e operação K11; o `README.md` raiz é o índice documental. | Documentação com valores de produção. Só atualizar no release de corte, mantendo histórico e caminho de rollback. |
| Contrato de controle | `docs/control-protocol-v1.md:44,53,71-95` | Documenta auth, relay, pin TLS, limites, DNS, estado e K11, mas trechos sobre payload/limites precisam ser conferidos contra o código atual. | Especificação operacional. Corrigir inconsistências no release de código antes da migração; não tratar prosa antiga como prova. |
| Visão desktop | `docs/desktop-technical.md:7-28` | Descreve persistência/configuração e ainda afirma limites/control-plane que conflitam com o relay em evolução. | Documentação de arquitetura. Reconciliar com implementação revisada antes do gate da Fase 1. |
| Plano de mídia | `docs/media-pion-plan.md` | Descreve a integração Pion e contém afirmações antigas de ausência de TURN/relay que precisam ser confrontadas com `media/cmd/harbor-media` e o core atuais. | Planejamento potencialmente obsoleto. Reconciliar configuração ICE, fallback e testes físicos antes de concluir a Fase 1. |
| Plano de rede direto/relay | (removido em 2026-09-08 com o resto: era `docs/network-plan.md`) | Registrava P2P, STUN, TURN, Harbor Relay, orçamentos e matriz física. | Planejamento absorvido por este documento; remoção executada por decisão do operador. |
| Plano Nebula | (removido em 2026-09-08 com o resto: `docs/network-nebula.md`, `docs/network-slice1.md`, `network/harbor-network/`) | Era planejamento experimental com endpoint K11/CGNAT e smoke opt-in. | Remoção executada por decisão do operador; sem fallback overlay a partir daqui. |
| Fixtures IPv4 | `core/harbor-core/src/app/mobile_link.rs:1435-1462`, `app/mod.rs:5114,5638-5685`, `server.rs:530-550` | Usa `192.168.1.6`, `127.0.0.1` e TEST-NET `192.0.2.1` em normalização, persistência e timeout. | Constantes genéricas de teste; não substituir pela Oracle. |
| Fixtures IPv6 | `core/harbor-core/src/server.rs:346-528`, `app/mod.rs:7647-7665`; `server/harbor-server/src/transport.rs:776-835` | Usa loopback, `[::1]` e documentação TEST-NET `[2001:db8::1]`. | Constantes genéricas de teste; não são o IPv6 do K11. |

## Arquitetura de destino

A ordem de transporte desejada é:

1. conexão P2P/direta, incluindo IPv6, IPv4 e hole punching, sempre preferida;
2. Oracle como plano de controle e rendezvous STUN;
3. TURN na Oracle quando o caminho direto de mídia não existir;
4. Harbor Relay como último fallback para tráfego Harbor compatível.

O servidor não deve manter banco de payload privado. Seu estado durável continua
limitado a identidades registradas e relacionamentos aceitos; presença, códigos e
sessões continuam transitórios. Payload relayado deve permanecer E2E cifrado, sem
logging de conteúdo. Em qualquer instante existe exatamente um servidor
autoritativo gravável.

## Portas, endereçamento e três camadas de firewall

O código atual compartilha `9091`: TCP para controle TLS e UDP para STUN-lite e
TURN em modo compatível. O modo externo (implementado, pendente de validação
física) adiciona sockets dedicados por alocação na faixa UDP configurável
`49160-49175` via `HARBOR_TURN_EXTERNAL_IP` + `HARBOR_TURN_RELAY_PORT_RANGE`.
Portanto, a abertura inicial permitida continua sendo somente `9091/tcp` e
`9091/udp`; a faixa de relay só abre no Gate 9 (canário por IP) após o código
abaixo ser validado externamente. Não abrir `3478`/`5349`. Abrir portas sem
consumidor validado aumenta a superfície.

SSH `22/tcp` deve ser restringido posteriormente a origens administrativas
explícitas. Antes dessa restrição, é obrigatório provar um segundo acesso
administrativo para evitar lockout. O login direto de root por chave deve ser
substituído por usuário administrativo sem privilégio permanente, com elevação
auditável, somente em fase aprovada.

As três camadas de firewall a auditar são:

1. **Security list da subnet OCI:** regras associadas à subnet da VNIC. Seu
   estado atual é desconhecido e deve ser consultado no plano de controle OCI.
2. **NSG da VNIC OCI:** regras dos NSGs efetivamente anexados à VNIC. O estado e
   até a existência de um NSG anexado são desconhecidos. NSG e security list são
   fontes aditivas da política OCI; não duplicar regras sem necessidade e revisar
   o conjunto efetivo para evitar permissão ampla acidental.
3. **Firewall do guest:** zona/interface corretas no `firewalld`, com regras
   explícitas por protocolo. SELinux permanece enforcing; qualquer negação deve
   ser corrigida por política, nunca por desabilitação.

Antes dessas camadas, o listener da aplicação deve estar em bind TCP e UDP no
endereço correto do guest. Um processo em loopback ou ausente continua
inacessível mesmo com os três firewalls permitindo tráfego.

Além dessas camadas, o IP público/NAT da OCI deve estar reservado e mapeado para
`10.0.0.7`. Esse mapeamento é pré-condição de estabilidade, não uma quarta regra
de firewall. Cada teste deve distinguir `refused` (caminho chegou sem listener)
de timeout/filtered (alguma camada bloqueou).

## Bloqueios confirmados de código

| Bloqueio | Evidência exata | Risco | Gate obrigatório |
| --- | --- | --- | --- |
| Tomada de chave de identidade | `server/harbor-server/src/lib.rs:459-474,641-665`; `control/src/lib.rs:151-180` | `identity.update` aceita chave autorreferendada e `register_identity()` substitui incondicionalmente o UUID conhecido. Quem conhece o UUID pode substituir a chave. | Re-registro de UUID existente com chave diferente deve ser rejeitado ou exigir autorização autenticada pela identidade anterior. Adicionar teste negativo e revisão de segurança antes de qualquer exposição pública. |
| TURN atrás de NAT/endereço público e roteamento | Antes: `stun.rs` anunciava `socket.local_addr()` (wildcard/`10.0.0.7`); roteamento ambíguo no socket compartilhado. Agora: `turn.rs` (`TurnRelayConfig`, `relay_receipt`, `allocation_socket`, orçamento 512 KiB/s, `peer_allowed`), `stun.rs` (readers dedicados, buffer 2048), `main.rs` (`HARBOR_TURN_EXTERNAL_IP` + `HARBOR_TURN_RELAY_PORT_RANGE` default `49160-49175`). Compat preservado sem env (K11). 12 testes TURN verdes + 56 server verdes, clippy limpo. | Sem o modo externo, Oracle anunciaria endereço indiscalizável e peers colidiriam no socket único. | Código implementado e testado localmente; falta T1/T2 (canário externo bidirecional por TURN, NAT rebinding) + revisão antes de abrir a faixa. Não contornar abrindo `3478`/range antes do Gate 9. |
| Rollback de nonce/sequence do Harbor Relay | `core/harbor-core/src/app/relay_crypto.rs:163-171`; `app/mod.rs:1581-1606`; `server/harbor-server/src/relay.rs:262-268` | Qualquer `relay_busy` decrementa `send_seq` sem vincular a resposta ao frame nem excluir envio posterior; concorrência ou resposta atrasada pode reutilizar nonce AEAD. | Tornar sequência monotônica ou rollback estritamente transacional por frame; testar envios concorrentes, busy atrasado e envio posterior. Revisão criptográfica obrigatória. |
| Migração de endpoint de clientes existentes | Implementado: `core/pairing.rs` (`LEGACY_K11_ENDPOINTS`, `migrate_server_pin`, atômico+idempotente), IPC `server.migrate` + `server.config.needs_migration` (`app/mod.rs`), `protocol` allowlist, `HarborFacade::migrateServerEndpoint` + `serverNeedsMigration` + env `HARBOR_SERVER_DEFAULT_ADDRESS`, mobile `migrateServer()` + `serverNeedsMigration`, placeholder LAN obsoleto trocado por exemplo neutro. Pin preservado, custom nunca sobrescrito. 223 testes core verdes (incl. `server_migrate_moves_legacy…`). | Sem migração, clientes antigos continuariam no K11 após o corte. | Código + testes locais verdes; falta validar release (E1-E5: upgrade/downgrade, instalação nova, custom, crash, sessão ativa) em Linux/Windows/Android físicos antes do corte em ondas. |

Achado relacionado a endurecer na Fase 1: `server/harbor-server/src/turn.rs:358-371`
valida validade do nonce TURN, mas não o consome; ele pode ser reutilizado até o
TTL. A política esperada deve ser decidida e coberta por teste antes da exposição.

## Endereço de produção (sem domínio)

Decisão: **sem DNS — endpoint é o IPv4 público cru da Oracle**
(`<oracle-ipv4>:9091`, mesmo pin `b9846…`). O valor real viaja por
`HARBOR_ORACLE_ADDRESS` / `HARBOR_TURN_EXTERNAL_IP` e nunca é compilado no
app; este documento usa o placeholder (o valor medido em 2026-09-07 fica no
terminal administrativo privado, fora do repositório).

Sem nomes, não há TTL, zona ou A/AAAA: o smoke por
IP (`server/oracle/smoke.sh <oracle-ipv4> <pin>`) é a validação. O K11 mantém
seu IPv6 como endereço de rollback; reversão de endpoint no cliente continua
sendo `server.configure` explícito (fallback de alcance).

Sequência autorizada:

1. confirmar no plano de controle OCI que `<oracle-ipv4>` é reservado e está
   corretamente associado à VNIC;
2. exportar `HARBOR_ORACLE_ADDRESS=<oracle-ipv4>:9091` e
   `HARBOR_TURN_EXTERNAL_IP=<oracle-ipv4>` no ambiente operador;
3. validar serviço, pin e `9091/tcp+udp` pelo IP a partir de rede externa
   (`smoke.sh` + mídia TURN bidirecional real);
4. distribuir a migração de clientes em ondas (`server.migrate`, segundo
   salto idêntico se o IP um dia mudar).

## Migração de certificado e estado

O certificado, a chave correspondente e o snapshot de controle formam a
identidade operacional a preservar. Como os clientes usam pin do certificado
completo, e não uma CA estável, renovação ACME ou qualquer regeneração altera o
fingerprint e rompe os clientes. Não iniciar ACME nem rotacionar o certificado
existente nesta migração.

Procedimento:

1. Antes da janela final, ensaiar cópia e restauração com material de teste ou
   backup cifrado, sem iniciar uma segunda autoridade pública.
2. Registrar tamanho, modo e SHA-256 completo dos três arquivos no terminal
   administrativo privado; nunca registrar conteúdo de `tls/key.pem`.
3. Na janela final, bloquear novas operações, encerrar de forma limpa o K11 e
   confirmar que TCP e UDP `9091` deixaram de atender. Esse é o início da
   quiescência.
4. Produzir o snapshot final de `$HOME/harbor/state/tls/cert.pem`,
   `$HOME/harbor/state/tls/key.pem` e
   `$HOME/harbor/state/state/control-state-v1.json`; transferi-lo por canal
   autenticado e cifrado, sem arquivos intermediários públicos.
5. Instalar no diretório Oracle definitivo com diretórios `0700`, arquivos
   `0600`, owner exclusivo do serviço e contexto SELinux correto.
6. Comparar tamanho e SHA-256 origem/destino; verificar offline que a chave
   corresponde ao certificado e que o fingerprint servido é exatamente
   `b9846aed2e97bd741ae5a2a3de9ab37c1831d2372ca67f26f538bd279dd7271f`.
7. Iniciar somente a Oracle, executar os gates e manter o K11 parado como unidade
   de rollback.

Rollback antes de qualquer escrita aceita na Oracle: parar Oracle, remover sua
exposição, confirmar portas fechadas, reiniciar e validar o K11 com o snapshot
original. Sem DNS, não há reversão de nome: clientes já migrados voltam ao K11
por `server.configure` explícito em ondas (somente fallback de alcance, nunca
automático silencioso).
Rollback depois de escrita aceita na Oracle não pode reativar o snapshot antigo
do K11. O procedimento determinístico é: bloquear clientes; parar e isolar a
Oracle; capturar seu snapshot final; manter o K11 parado; fazer backup da árvore
K11; substituir integralmente no K11 os três arquivos autoritativos pela versão
Oracle; restaurar owner/modos; comparar hashes; verificar chave/certificado,
fingerprint e compatibilidade de schema/binário; iniciar somente o K11; e executar
smoke e um relacionamento existente. Depois de as escritas Oracle estarem
quiescidas e seu estado restaurado no K11, clientes já migrados voltam ao
endpoint K11 por `server.configure` explícito em ondas (sem DNS não há
reversão de nome a propagar). Nunca fazer
merge manual de JSON nem iniciar ambos para "sincronizar". Se a restauração ou
compatibilidade falhar, ambos permanecem parados e isolados; restaura-se o backup
pré-tentativa do K11 apenas para nova tentativa. Voltar ao snapshot pré-corte,
descartando escritas Oracle, exige aprovação explícita de perda de dados. Códigos,
presença e sessões em voo são transitórios e podem ser perdidos; clientes devem
reconectar.

## Proposta de serviço, armazenamento, logs e backup

Esta seção é uma proposta adequada ao guest de 498 MiB; **nada aqui está
implantado**.

- Usuário de sistema dedicado, sem shell e sem login; binário root-owned e não
  gravável pelo serviço.
- Estado em `/var/lib/harbor-server`, com `StateDirectory=harbor-server`, modo
  `0700`; `tls/*.pem` e `state/control-state-v1.json` em `0600`.
- Configuração não secreta em `/etc/harbor-server/harbor-server.env`, `0600`,
  contendo bind e caminhos. Nenhuma chave, token OCI ou URL DDNS em argumentos ou
  logs.
- Unidade `Type=simple`, `Restart=on-failure`, `RestartSec=5s`,
  `StartLimitIntervalSec=60`, `StartLimitBurst=5`, `TimeoutStopSec=15s`.
- Limites iniciais conservadores: `MemoryHigh=160M`, `MemoryMax=192M`,
  `TasksMax=64`, `LimitNOFILE=4096`. Medir RSS, swap, OOM e latência; ajustar
  somente com evidência, considerando os ~70 MiB do OCI agent.
- Hardening a validar: `NoNewPrivileges=yes`, `PrivateTmp=yes`,
  `ProtectSystem=strict`, `ProtectHome=yes`, `ProtectKernelTunables=yes`,
  `ProtectKernelModules=yes`, `ProtectControlGroups=yes`,
  `RestrictSUIDSGID=yes`, `LockPersonality=yes` e escrita somente no state dir.
  Não aplicar opção que impeça sockets necessários sem um teste em staging.
- Logs apenas no journal, sem payload, chave, token, código de pareamento ou
  envelope privado. Registrar startup, versão/hash do binário, bind, fingerprint
  público, contadores, erros categorizados e transições de saúde. Propor limite
  persistente total de 128 MiB e retenção de 14 dias, verificando impacto nos
  demais serviços antes de alterar a configuração global do journald.
- Monitorar `systemctl is-active`, listeners TCP/UDP, reinícios, RSS/swap, espaço
  e inodes, falhas TLS/STUN/TURN/relay e AVCs SELinux. Alertar antes de 80% de
  disco e em qualquer OOM/restart loop.
- Backup consistente após quiescência ou mecanismo de snapshot comprovado:
  certificado, chave e `control-state-v1.json`, cifrados antes de sair do host,
  com acesso mínimo, checksum autenticado, retenção definida e pelo menos um
  restore offline testado. Não incluir journal nem payload. Backup não substitui
  o snapshot final nem autoriza duas autoridades.

## Plano operacional — Fases 0 a 18

Em todas as fases, "logs" significa evidência com timestamp UTC, versão/hash,
origem do teste e resultado, sem segredos. O responsável registra aprovação do
gate antes da fase seguinte.

| Fase | Execução | Testes e logs exigidos | Gate de saída | Rollback |
| ---: | --- | --- | --- | --- |
| 0 — auditoria | Consolidar inventário Oracle/K11, dependências, arquitetura, riscos e plano, sem mudanças remotas. | Revisão manual deste Markdown; conferir fatos e caminhos; `git diff --check`. Log: diff local e resultado da validação. | **Concluída** quando este documento é revisável e é a única alteração intencional desta tarefa. | Remover apenas este arquivo; nenhum sistema operacional foi alterado. |
| 1 — bloqueios de código | Corrigir tomada de identidade, TURN NAT/public address/rotas, rollback de nonce relay e migração de endpoint; decidir replay de nonce TURN. | Unitários negativos e concorrentes; testes de integração externo-NAT; Rust fmt/clippy/test; revisão de segurança/cripto. Logs: resultados completos e commit/artifact hash. | Código dos quatro bloqueios implementado e verde localmente (identidade + relay aprovados por Terra; TURN + endpoint revisados em self-review do orquestrador em 2026-09-08: 5 achados menores corrigidos, 14 testes TURN + 223 core verdes, clippy limpo nos crates tocados). Gate físico externo (T1/T2/E1-E5) continua obrigatório antes de qualquer exposição. | Não distribuir build; K11 e clientes permanecem na versão/endpoint atuais. |
| 2 — artefato Oracle | Produzir binário reprodutível `x86_64` a partir de revisão aprovada; SBOM/checksum se suportado. | Executar suíte server/core/protocol; iniciar com estado descartável em loopback; verificar hash e dependências. Logs: CI e SHA-256 completo. | Release `harbor-server` instalado em 2026-09-08, SHA-256 `7ed60749…` (verificado origem/destino/guest, inalterado), suítes verdes; SBOM CycloneDX 1.7 (Syft 1.51.1, 0 packages) em `/tmp/harbor-server.sbom.json`, SHA-256 `6fc142c1…` — mover para local persistente antes de reboot. | Descartar artefato; nenhuma implantação. |
| 3 — plano de controle OCI | Verificar reserva de `<oracle-ipv4>`, VNIC/subnet, NSG/security list, rotas e backup/console de acesso. Não inferir do guest. | CONFIRMADO 2026-09-08 (operador): Reserved Public IP; NSG + firewalld com TCP 9091, UDP 9091, UDP 49160-49175; smoke externo verde. | IP comprovadamente reservado e regras propostas, ainda mínimas. | Reverter qualquer regra OCI criada nesta fase. |
| 4 — administração e host | Criar acesso administrativo nominal, testar elevação, planejar restrição de SSH, manter SELinux/chronyd/firewalld ativos. | Duas sessões independentes; reboot controlado; auditoria de `sshd`, tempo, AVC e zona. Logs: sucesso sem chaves. | Acesso não-root recuperável e console/rollback confirmados antes de restringir 22. | Restaurar configuração SSH anterior pela sessão/console; não tocar no Harbor. |
| 5 — layout e unidade staging | Criar usuário, diretórios e unidade systemd proposta, inicialmente em loopback e com estado de teste. | `systemd-analyze verify`; start/stop/restart/reboot; permissões, SELinux, limites, journal e OOM ausente. Logs: `systemctl status`, journal sanitizado, RSS. | Serviço de teste estável por 24 h dentro dos limites; sem estado K11. | Desabilitar/remover unidade e estado de teste; deixar portas públicas fechadas. |
| 6 — ensaio de backup/restore | Ensaiar transporte cifrado, checksum, restore e correspondência cert/key com dados de teste ou cópia offline protegida. | Comparar modos/tamanhos/hashes; restore em diretório isolado; teste de perda/corrupção. Logs: checksums autorizados, nunca conteúdo. | Runbook reproduzível por duas pessoas e restore comprovado. | Destruir cópias de teste segundo política; K11 não para. |
| 7 — validação funcional isolada | Executar artefato Oracle com identidade/estado descartáveis, sem ingress público e sem usar a identidade autoritativa do K11. | TLS pin de teste, protocolo controle, STUN, TURN com endereço simulado, relay E2E, restart e persistência. Logs: matriz automatizada e journal. | Verde no guest em 2026-09-08 (loopback, identidade descartável, compat + dry-run externo TEST-NET, smoke 2× OK, systemd transient como `harbor-server` OK); relay E2E físico e 24h de estabilidade pendentes. | Parar serviço e apagar somente estado descartável. |
| 8 — firewall staging | Preparar regras `9091/tcp+udp` restritas a IPs de teste nas três camadas; manter `3478/5349/range` fechadas. | De origem permitida e negada, testar TCP e UDP separadamente; confirmar bind e counters. Logs: origem/protocolo/resultado. | Evidência 2026-09-08: TCP 9091 externo OK (pin staging `73f24c…`, ≠ K11 — correto); UDP 9091 estava ausente no firewalld (só runtime) e foi fixado permanente (`9091/tcp 9091/udp 49160-49175/udp`); smoke externo verde (STUN echo + TURN 401); matriz completa após T1. SSH continua acessível. | Remover regras OCI/firewalld de 9091; bind volta a loopback. |
| 9 — canário por IP | Com estado descartável e janela aprovada, validar `<oracle-ipv4>:9091` de rede externa; não aceitar clientes de produção. | TLS/control, STUN reflexivo, TURN bidirecional real e relay; verificar anúncio público, não `10.0.0.7`/wildcard. Logs: pcap sanitizado/metadados e journal. | Todos os caminhos externos de canário verdes; bloqueio TURN comprovadamente resolvido. | Remover ingress de teste da security list, de todos os NSGs anexados e do `firewalld`; parar o listener; comprovar TCP+UDP fechados externamente; preservar evidência. |
| 10 — endpoint sem DNS | Sem domínio por decisão: confirmar `HARBOR_ORACLE_ADDRESS=<oracle-ipv4>:9091` único em todos os ambientes operador e `HARBOR_TURN_EXTERNAL_IP=<oracle-ipv4>` no servidor. | `server.migrate` com e sem `to_address` contra pin legado; `server.config` com `migration_target` preenchido. | Endpoint único documentado; migração em dois saltos testada. | Remover a variável dos ambientes operador. |
| 11 — release de clientes | Oracle-default builds via `HARBOR_ORACLE_ADDRESS` em tempo de build (desktop `CMakeLists`, mobile `Harbor-Mobile/CMakeLists` + `oracleDefaultAddress`, CI `build.yml` lendo a repo Variable — vazio = K11, como no CI sem a variável); migração idempotente do endpoint antigo exato para `<oracle-ipv4>:9091` (via env ou `to_address`), sem alterar pin; preservar customizações e reconectar tudo. | Mecanismo provado localmente: define chega ao target, objeto contém default + fallback K11, QML respeita override. Upgrade/downgrade, instalação nova, config customizada, app aberto/fechado, chamada ativa, restart e corrupção de state. Logs: versões e transições sem conteúdo. | Builds Linux/Windows/Android aprovados; caminho de rollback e telemetria não sensível testados. | Retirar release; manter endpoint K11 e restaurar versão cliente anterior. |
| 12 — canário de cliente novo | Cliente de teste novo usa `<oracle-ipv4>:9091` via `HARBOR_ORACLE_ADDRESS`, sem tocar em estado de produção. | Pairing, presença, chat, arquivo, chamada, direct/STUN/TURN/relay e pin errado. Logs: test IDs e caminho escolhido. | Nenhuma dependência residual do literal K11 no caminho canário. | Remover env e estado do cliente canário. |
| 13 — Android físico | Executar matriz em aparelho físico, redes Wi-Fi, móvel, IPv4-only e IPv6 quando disponível; testar background/restart. | Casos A da matriz, incluindo upgrade com `server-pin-v1.json` existente, chamada e relay. Logs: modelo/OS/rede/build e resultados. | Todos os casos Android obrigatórios verdes. | Voltar app/config ao K11; sem corte. |
| 14 — Windows físico | Instalar e atualizar pacote em Windows físico limpo; validar firewall local, suspensão/rede e ausência de dependência removida. | Casos W da matriz, instalador/upgrade/uninstall, direct/TURN/relay e pin. Logs: versão Windows/installer/hash. | Todos os casos Windows obrigatórios verdes. | Desinstalar/voltar pacote anterior e endpoint K11. |
| 15 — CGNAT e falhas | Provar dois peers físicos atrás de CGNAT distintos, ausência de caminho direto e fallbacks; injetar perda/reordenação/restart. | Casos N/R da matriz; áudio, DataChannel, chat e arquivo; monitorar sequência/nonce e vazamento. Logs: NAT detectado, caminho, latência/perda. | P2P vence quando possível; TURN e Harbor Relay funcionam sem nonce reuse nem payload no servidor. | Encerrar canários, fechar 9091 público e manter K11 autoridade. |
| 16 — snapshot final e quiescência | Aprovar manutenção, impedir novas operações, parar K11, verificar portas inativas, capturar snapshot final e instalar na Oracle ainda isolada. | EXECUTADO 2026-09-08T03:43–44Z: supervisor+servidor K11 parados por PID (boot desativado reversível), TCP+UDP 9091 down, snapshot `cert/key/control-state-v1.json` com hashes idênticos origem/destino (`b04058…`/`fb185…`/`6e780f…`), owner/modos/SELinux OK, trânsito destruído no guest. | K11 parado; cópia Oracle íntegra; nenhuma escrita aceita por qualquer servidor. | Se Oracle ainda não escreveu: isolar Oracle e reiniciar K11 original. |
| 17 — corte de autoridade | Iniciar Oracle como única autoridade, abrir somente 9091 TCP+UDP, validar por IP. Distribuir/ativar migração de clientes em ondas. | AUTORIDADE LIGADA 2026-09-08T03:44:29Z (unit enabled): serve pin produção `b9846…` (match completo), smoke externo verde. PENDENTE: ondas de migração nos aparelhos (E1–E5 físicos) + matriz crítica pós-corte. | Oracle única e estável; clientes críticos conectados; zero listener K11. | Antes de escrita, isolar Oracle, iniciar e validar o K11 original. Depois de escrita, parar/isolar Oracle, restaurar integralmente seu snapshot no K11 parado, validar e iniciar só K11. Rollback de endpoint no cliente é `server.configure` explícito em ondas. Falha mantém ambos isolados. |
| 18 — estabilização | Observar por no mínimo 7 dias (janela: 2026-09-08T03:44Z → ~2026-09-15T03:44Z), testar backup/restore offline; manter K11 parado e disponível para rollback. | Matriz diária crítica, recursos, logs, backup restaurado e auditoria de firewall e incidentes. | SLO operacional acordado, restore comprovado, documentação final aprovada; somente então encerrar migração. | Durante a janela, usar a regra pós-escrita da Fase 17: restaurar apenas no K11 isolado. Rollback de endpoint no cliente é apenas fallback de alcance. K11 é backup frio (parado, boot desativado); sem caminhos VPN a partir da remoção de 2026-09-08. |

## Matriz completa de testes de migração

Nenhuma linha manual pode ser marcada como aprovada sem data, executor, origem,
build/hash, endpoint resolvido, transporte realmente escolhido e evidência. Para
testes de conteúdo, registrar apenas hash/tamanho e resultado, nunca payload.

| ID | Ambiente e pré-condição | Procedimento | Aprovação/evidência | Obrigatório em |
| --- | --- | --- | --- | --- |
| B1 | Oracle isolada, estado descartável | Start, stop, SIGTERM, restart e reboot. | Uma instância, sem órfão; TCP+UDP retornam; state consistente; journal sem segredo. | Fases 5/7 |
| B2 | 498 MiB RAM + swap reais | Controle, STUN, TURN e relay sob carga limitada. | Sem OOM/thrash; RSS, swap, CPU e latência dentro dos limites aprovados. | Fases 7/9/18 |
| C1 | Cert/key migrados offline | Comparar hash, chave/cert, CN, validade e fingerprint servido. | Fingerprint completo `b984…` exato; modos `0600`; nenhuma chave em log. | Fase 16 |
| C2 | Cliente com pin correto/incorreto | Conectar com cada pin. | Correto aceita; incorreto falha fechado, sem fallback inseguro. | Fases 7/12/17 |
| S1 | Snapshot final | Validar JSON, owner/modo/hash e restart após escrita controlada. | Identidades/relações preservadas; escrita atômica; códigos/sessões não ressuscitam. | Fases 16/17 |
| I1 | UUID existente/chave diferente | Tentar `identity.update` autorreferendado. | Rejeitado sem alterar estado; evento auditável sem chave. | Fase 1 |
| I2 | UUID novo e atualização autorizada | Registrar e executar fluxo legítimo definido. | Funciona sem reabrir takeover. | Fase 1 |
| E1 | Instalação nova por plataforma | Iniciar sem `server-pin-v1.json`. | Endpoint `<oracle-ipv4>:9091` e pin exato persistidos `0600`. | Fases 11–14 |
| E2 | Instalação com literal K11 exato | Atualizar cliente. | Migra uma vez, reconecta controle/STUN/TURN/relay e preserva identidade local. | Fases 11–14 |
| E3 | Endpoint customizado | Atualizar cliente. | Configuração não é sobrescrita. | Fases 11–14 |
| E4 | Migração interrompida/crash | Interromper antes/durante escrita e reiniciar. | Estado antigo ou novo válido; nunca JSON parcial; segunda execução idempotente. | Fase 11 |
| E5 | Sessão/chamada ativa | Acionar mudança de endpoint durante atividade. | Comportamento definido: adiar ou reconectar com erro claro; sem envio a duas autoridades. | Fase 11 |
| D1 | Endpoint antes do corte | Confirmar `HARBOR_ORACLE_ADDRESS` único nos ambientes operador. | Mesmo valor em cliente canário, servidor e runbook. | Fases 10/17 |
| D2 | Endpoint ausente/inválido | Remover a variável ou invalidar o valor. | `server.migrate` recusa com `invalid_request`; sem fallback silencioso ao K11 depois do corte. | Fase 12 |
| F1 | Origem administrativa e pública | Provar 22, 9091 TCP, 9091 UDP, 3478, 5349 e range. | 22 só conforme política; 9091 TCP+UDP conforme fase; demais fechadas. | Fases 8/17/18 |
| F2 | SELinux enforcing | Exercitar todos os caminhos e revisar AVC. | Zero negação não resolvida; SELinux nunca permissive/disabled. | Fases 5/9/17 |
| N1 | Internet IPv4 externa | Controle TLS pelo IP Oracle. | Conecta ao IP Oracle, pin correto, sem endereço privado anunciado. | Fases 9/17 |
| N2 | STUN UDP 9091 | Solicitar endpoint reflexivo de múltiplas redes. | IP/porta observados corretos; malformed recusado. | Fases 9/15/17 |
| N3 | P2P disponível | Parear dois peers LAN, IPv4 público ou IPv6 real do cliente. | Direto é selecionado antes de relay; servidor só sinaliza. | Fases 12–15 |
| N4 | Dois CGNAT distintos, peers físicos | Parear e forçar falha direta. | Fallback via TURN/relay Oracle; caminho reportado corretamente. | Fase 15 |
| T1 | TURN Oracle atrás do mapeamento público | Allocate/permission/send/receive nos dois sentidos. | Evidência 2026-09-08 (`server/harbor-server/tests/t1_oracle.rs`, staging): 2 dispositivos provisionados via `turn.credentials`, relay anunciado `<oracle-ipv4>:49160` + `<oracle-ipv4>:49161` (sockets dedicados, nunca wildcard/privado), mídia íntegra ida e volta — incl. hairpin mesma-relay (own-source bypass). Redes físicas/CGNAT distintas pendentes. | Fases 9/15/17 |
| T2 | NAT rebinding e famílias | Mudar porta de origem; testar IPv4 e IPv6 do peer quando disponível. | Comportamento suportado é determinístico, autenticado e sem alocação cruzada. | Fases 1/15 |
| T3 | Credencial/nonce inválido, expirado e repetido | Repetir transações e avançar relógio controlado. | 401/438/silêncio conforme protocolo e política aprovada; sem replay indevido. | Fases 1/7 |
| R1 | Harbor Relay E2E | Chat/perfil e arquivo conhecido via relay forçado. | Servidor observa só ciphertext/metadados; hashes finais iguais. | Fases 7/15 |
| R2 | Relay concorrente/busy atrasado | Enviar frames concorrentes, encher fila e atrasar resposta. | Nenhum nonce/sequence reutilizado; receiver rejeita replay sem corromper sessão. | Fase 1 |
| R3 | Arquivo 100 MiB | Transferir, interromper, alternar direto↔relay e retomar. | SHA-256 igual, sem reinício indevido ou duplicação. | Fase 15 |
| M1 | Chamada física | Áudio + DataChannel direto, TURN e misto. | Áudio bidirecional estável, DataChannel íntegro e caminho evidenciado. | Fases 13–15 |
| A1 | Android físico Wi-Fi/móvel | Instalação nova, upgrade, background, force-stop e troca de rede. | Endpoint migra, reconecta e todas as funções críticas passam. | Fase 13 |
| W1 | Windows físico limpo | Instalar, atualizar, reboot, suspensão e troca de rede/firewall. | Pacote íntegro, sem dependência externa inesperada, direct/TURN/relay passam. | Fase 14 |
| P1 | Relação já pareada no snapshot | Conectar os dois dispositivos após corte. | Relação continua aceita; não exige novo pairing. | Fase 17 |
| P2 | Pairing novo pós-corte | Criar, submeter, aceitar e reiniciar servidor. | Relação persiste somente na Oracle e sobrevive ao restart. | Fase 17 |
| X1 | Queda Oracle durante operação | Parar serviço/rede e restaurar. | Cliente falha de modo claro e reconecta; nenhuma escrita vai ao K11 parado. | Fases 15/18 |
| X2 | Rollback pré-escrita | Falhar antes da primeira mutação Oracle com um cliente já migrado; voltar o cliente ao K11 por `server.configure` em ondas. | Oracle isolada; K11 volta com hash original; cliente reconecta ao endpoint K11. | Ensaio antes da Fase 17 |
| X3 | Rollback pós-escrita | Com estado descartável, parar/isolar Oracle, copiar integralmente seu snapshot para K11 parado, validar e iniciar só K11. | Relações/escritas Oracle aparecem no K11; hashes/schema/pin passam; falha deixa ambos isolados; nunca dois writers. | Ensaio antes da Fase 17 |
| O1 | Backup Oracle | Backup cifrado e restore em host/diretório isolado. | Checksums e serviço restaurado passam C1/S1; acesso e retenção auditados. | Fase 18 |
| O2 | Observabilidade | Gerar falhas TLS, STUN, TURN, relay, disco e restart. | Alertas acionam; logs bastam para diagnóstico e não contêm segredo/payload. | Fases 7/18 |

## Critério final

A migração só termina após o gate da Fase 18. Até lá, o K11 continua sendo a
autoridade (antes do corte) ou o rollback preservado e desligado (depois do
corte). A ausência de teste físico Android, Windows ou CGNAT mantém o plano
bloqueado, mesmo que toda a automação esteja verde.
