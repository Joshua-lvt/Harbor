/**
 * Pairing screen — the "Install → Connect → Done" entry experience.
 *
 * Shows this device's pairing code (with Copy), and an input to paste a
 * partner's code. After a successful /pair, persists the partner link and the
 * parent reroutes to chat.
 */
import { useEffect, useRef, useState } from "react";
import { register, pair, setProfile } from "../../lib/relay";
import { loadIdentity, saveIdentity } from "../../lib/identity";
import { usePassivePairing, withPartner } from "../../hooks/usePassivePairing";
import { genKeypair } from "../../lib/crypto";
import type { Identity } from "../../lib/types";
import { SharkMascot } from "../../assets/shark";

export default function PairingScreen({
  identity,
  onPaired,
}: {
  identity: Identity;
  onPaired: (next: Identity) => void;
}) {
  const [code, setCode] = useState(identity.pairing_code ?? "");
  const [secret, setSecret] = useState(identity.device_secret ?? "");
  const [registering, setRegistering] = useState(false);
  const [partnerCode, setPartnerCode] = useState("");
  const [myName, setMyName] = useState(identity.my_name ?? "");
  const [status, setStatus] = useState<string>("");
  const [busy, setBusy] = useState(false);

  // Ensure we have a code to show (registers if first launch / store wiped).
  // The X25519 keypair is generated FIRST, so the pubkey travels inside the
  // /register request and the relay stores it for the future partner to fetch.
  useEffect(() => {
    (async () => {
      if (!identity.pairing_code || !identity.device_secret) {
        setRegistering(true);
        try {
          const kp = await genKeypair();
          const res = await register(identity.relay_url, identity.device_id, kp.pub);
          setSecret(res.device_secret);
          await saveIdentity({
            ...identity,
            pairing_code: res.pairing_code,
            device_secret: res.device_secret,
            device_pubkey: kp.pub,
            device_privkey: kp.priv,
          });
          setCode(res.pairing_code);
        } catch (e) {
          setStatus(`Não foi possível contatar o servidor: ${(e as Error).message}`);
        } finally {
          setRegistering(false);
        }
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Shared concurrency guard: the active paste-pair path (handlePair) and the
  // passive-detection polling (usePassivePairing) both acquire this before
  // completing a pairing. If the partner pairs with us from their device at the
  // same moment we click "Conectar", only one side runs; the other no-ops.
  const pairingRef = useRef(false);
  // Live credentialed identity for passive detection. The `identity` prop may
  // lag: register above persists the secret to the store + local `secret`
  // state but does not re-raise it to the parent, so `identity.device_secret`
  // can stay "". Thread the live `secret` in — exactly like handlePair builds
  // its own. Polling only starts once device_secret is non-empty (/me 401s
  // without it).
  const credentialed: Identity = { ...identity, device_secret: secret };
  usePassivePairing({ credentialed, pairingRef, onPaired });

  async function handlePair() {
    if (!partnerCode.trim() || !secret) return;
    if (pairingRef.current) return; // passive detection already completing this pairing
    pairingRef.current = true;
    setBusy(true);
    setStatus("");
    try {
      if (myName.trim()) await setProfile(credentialed, myName.trim());
      const res = await pair(credentialed, partnerCode.trim().toUpperCase());
      // Read the persisted identity fresh from the store — NOT the `identity`
      // prop (or `credentialed`). The prop never received the X25519 keypair
      // generated during /register (register persists it to the store + local
      // `secret` state but does not re-raise it to the parent), so building
      // `next` from the prop would OMIT device_pubkey/device_privkey, and the
      // integral saveIdentity write would wipe the keys the registration just
      // stored. Loading fresh guarantees the keypair travels into `next`, so
      // E2E is live immediately after pairing (no cold-start upgrade needed).
      let stored = null;
      try {
        stored = await loadIdentity();
      } catch {
        // local store read failed — keep whatever `credentialed` (prop + live
        // secret) carries; behavior matches pre-fix for the missing-keytail.
      }
      // The shared withPartner mapping sets partner_id/name/pubkey/avatar from
      // the /pair response (the partner's published key is learned at pair
      // time). my_name is the active path's own concern — it also publishes the
      // name via setProfile above — so it's layered on top here.
      const base: Identity = { ...(stored ?? credentialed), device_secret: secret };
      const next: Identity = {
        ...withPartner(base, res.partner_device_id, res),
        my_name: myName.trim() || null,
      };
      await saveIdentity(next);
      onPaired(next);
    } catch (e) {
      setStatus(`Falha ao conectar: ${(e as Error).message}`);
    } finally {
      setBusy(false);
      pairingRef.current = false; // release (no-op if we navigated away on success)
    }
  }

  return (
    <div className="window-main h-screen flex flex-col items-center justify-center gap-6 px-6 text-center">
      <div className="flex flex-col items-center gap-2">
        <SharkMascot className="w-20 h-20" />
        <h1 className="text-2xl font-semibold text-harbor-deep">Harbor</h1>
        <p className="text-sm text-harbor-ink/70">Seu porto seguro digital.</p>
      </div>

      <div className="w-full max-w-xs rounded-2xl bg-harbor-surface/70 backdrop-blur p-4 shadow-sm">
        <p className="text-xs uppercase tracking-wide text-harbor-deep/60">Seu código</p>
        {registering ? (
          <p className="mt-2 text-sm text-harbor-ink/60">Gerando…</p>
        ) : (
          <div className="mt-2 flex items-center justify-between gap-2">
            <code className="text-lg font-mono font-semibold text-harbor-deep">{code || "—"}</code>
            <button
              className="text-xs rounded-lg bg-harbor-sea text-white px-2.5 py-1.5 hover:bg-harbor-deep transition"
              onClick={async () => {
                if (code) try { await navigator.clipboard.writeText(code); setStatus("Copiado!"); } catch { setStatus("Não foi possível copiar."); }
              }}
            >
              Copiar
            </button>
          </div>
        )}
      </div>

      <div className="w-full max-w-xs flex flex-col gap-2">
        <details className="rounded-xl bg-harbor-sky/30 px-3 py-2 text-left">
          <summary className="cursor-pointer text-xs font-medium text-harbor-deep/70 list-none">
            Sobre o servidor 💡
          </summary>
          <p className="mt-1.5 text-[11px] leading-relaxed text-harbor-ink/70">
            O servidor (relay) é <strong>privado — seu e do seu parceiro, não nosso</strong>.
            Por padrão usamos <code>localhost</code> para teste. Para usar em redes
            diferentes, suba um servidor próprio e troque o endereço em Configurações
            (use <code>wss://</code> com TLS nesse caso).
          </p>
        </details>

        <input
          value={myName}
          onChange={(e) => setMyName(e.target.value)}
          placeholder="Seu nome (opcional)"
          className="w-full rounded-xl border border-harbor-sky bg-harbor-surface-strong px-3 py-2 text-sm outline-none focus:border-harbor-sea"
        />
        <input
          value={partnerCode}
          onChange={(e) => setPartnerCode(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handlePair()}
          placeholder="Cole o código do parceiro"
          className="w-full rounded-xl border border-harbor-sky bg-harbor-surface-strong px-3 py-2 text-sm font-mono outline-none focus:border-harbor-sea"
        />
        <button
          disabled={busy || !partnerCode.trim()}
          onClick={handlePair}
          className="w-full rounded-xl bg-harbor-deep text-white py-2.5 text-sm font-medium hover:bg-harbor-sea disabled:opacity-50 transition"
        >
          {busy ? "Conectando…" : "Conectar"}
        </button>
        {status && <p className="text-xs text-harbor-ink/70">{status}</p>}
      </div>

      <button
        className="text-xs text-harbor-deep/70 hover:underline"
        onClick={() => (window.location.hash = "#/settings")}
      >
        Configurações
      </button>
    </div>
  );
}
