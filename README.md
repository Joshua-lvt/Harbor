<p align="center">
  <img src="packaging/icons/hicolor/256x256/apps/harbor.png" width="160" alt="Harbor — um tubarão em águas calmas" />
</p>

<h1 align="center">Harbor</h1>

<p align="center"><em>Um porto seguro para duas pessoas se encontrarem — voz, conversa e presença, sem plateia.</em></p>

Harbor é um cantinho privado para você e alguém importante: uma chamada com
um toque, uma conversa que é só de vocês, os arquivos que precisam ir de um
lado para o outro e aquela tranquilidade de saber se a pessoa está por perto
— tudo direto entre os dois aparelhos, sem rede social, sem feed, sem anúncio
e sem ninguém olhando por cima do ombro.

## O que dá para fazer

- **Chamar** — voz com um toque, com push-to-talk e ativação por voz para quem prefere.
- **Conversar** — mensagens e arquivos direto de um aparelho para o outro, com confirmação de entrega.
- **Sentir presença** — online, ausente ou offline, e o que a pessoa está fazendo agora (se ela quiser mostrar).
- **Levar no bolso** — o Harbor Mobile estende tudo para o celular: mesma conversa, mesma chamada, e o PC mostra bateria, localização e notificações do telefone (tudo opt-in, tudo desligável).
- **Deixar do seu jeito** — modo claro/escuro, cores de destaque, fundos oceânicos, tudo aplicado na hora.

## Três versões, uma conta de si mesmas

| Versão | Estado |
| --- | --- |
| PC — Linux | Pronta para o dia a dia |
| PC — Windows | Pronta para o dia a dia |
| Android | 🚧 Em testes — funciona, mas ainda estamos lapidando |

Todas **se atualizam sozinhas**: quando sai uma versão nova aqui no GitHub,
o app baixa, confere a integridade e instala — sem botão de "depois", sem
versão velha para trás. Sem internet na hora? Ele segue funcionando e tenta
de novo sozinho.

## Como começa

1. Instale o Harbor nos dois lados.
2. Um lado mostra um código de 6 dígitos, o outro digita. Pronto — pareados.
3. Nenhum endereço, senha ou detalhe técnico aparece nunca: o servidor é
   infraestrutura invisível.

## Privacidade de verdade, não de marketing

- Voz, mensagens, arquivos, tela e localização **nunca passam pelo servidor** —
  vão de aparelho para aparelho, e a voz nem chega a existir fora da chamada.
- O servidor só apresenta as duas pontas (e nem isso ele enxerga por dentro).
- Tudo que é compartilhado tem um interruptor — e desligar remove de verdade.

## Para quem constrói

- Visão técnica do desktop: [`docs/desktop-technical.md`](docs/desktop-technical.md)
- Protocolo de controle: [`docs/control-protocol-v1.md`](docs/control-protocol-v1.md)
- Operação do servidor: [`docs/k11-runbook.md`](docs/k11-runbook.md)
- Arquitetura mobile: [`Harbor-Mobile/docs/architecture.md`](Harbor-Mobile/docs/architecture.md)
- Versão atual: [`VERSION.txt`](VERSION.txt) (única para PC, mobile e releases)
