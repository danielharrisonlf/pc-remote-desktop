import { useEffect, useRef, useState } from 'react';
import { toDataURL } from 'qrcode';
import { CircleCheck, Clock, LoaderCircle, RefreshCw } from 'lucide-react';
import { desktopApi } from '../api';
import { useNow } from '../hooks/useNow';
import { formatCountdown, groupCode } from '../lib/format';
import { preferredAddress, type DesktopStatus } from '../types';
import { Button } from './Button';
import { CopyButton } from './CopyButton';
import { Dialog } from './Dialog';
import { PendingRequest } from './DeviceItems';
import { FingerprintStrip } from './Fingerprint';

/** Matches `Duration::from_secs(120)` in src-tauri/src/pairing.rs. */
const PAIRING_WINDOW_MS = 120_000;

type Run = (key: string, action: () => Promise<unknown>, success?: string) => Promise<boolean>;

interface PairingDrawerProps {
  open: boolean;
  onClose: () => void;
  status: DesktopStatus | null;
  run: Run;
  isBusy: (key: string) => boolean;
}

export function PairingDrawer({ open, onClose, status, run, isBusy }: PairingDrawerProps) {
  return (
    <Dialog
      kind="drawer"
      open={open}
      onClose={onClose}
      title="Parear celular"
      description="O código vale por dois minutos e só funciona na mesma rede local."
    >
      {status ? <PairingContent status={status} run={run} isBusy={isBusy} onDone={onClose} /> : <p className="muted">Aguardando o serviço local…</p>}
    </Dialog>
  );
}

function PairingContent({ status, run, isBusy, onDone }: { status: DesktopStatus; run: Run; isBusy: (key: string) => boolean; onDone: () => void }) {
  const now = useNow(500);
  const pairing = status.pairing;
  const active = pairing !== null && pairing.expiresAt > now;
  const [qr, setQr] = useState<string | null>(null);
  const [authorized, setAuthorized] = useState<string | null>(null);
  const autoStarted = useRef(false);
  const generating = isBusy('pair');
  const address = preferredAddress(status.addresses);

  // Opening the drawer is the intent to pair: create a code right away unless a
  // valid one is still running (regenerating would invalidate a pending phone).
  useEffect(() => {
    if (autoStarted.current) return;
    autoStarted.current = true;
    if (!status.pairing || status.pairing.expiresAt <= Date.now()) void run('pair', desktopApi.startPairing);
  }, [run, status.pairing]);

  useEffect(() => {
    if (!pairing?.qrPayload) {
      setQr(null);
      return;
    }
    let current = true;
    toDataURL(pairing.qrPayload, { width: 464, margin: 1, errorCorrectionLevel: 'M', color: { dark: '#0f172a', light: '#ffffff' } })
      .then(data => current && setQr(data))
      .catch(() => current && setQr(null));
    return () => {
      current = false;
    };
  }, [pairing?.qrPayload]);

  const decide = (id: string, name: string, approved: boolean) => {
    void run(`decide:${id}`, () => desktopApi.approvePairing(id, approved), approved ? undefined : 'Pedido recusado.').then(ok => {
      if (ok && approved) setAuthorized(name);
    });
  };

  if (authorized) {
    return (
      <div className="pair-success">
        <span className="pair-success-icon" aria-hidden>
          <CircleCheck />
        </span>
        <p className="pair-success-title">{authorized} foi autorizado</p>
        <p className="muted">O celular já pode controlar este PC. Você pode revogar o acesso a qualquer momento em Dispositivos.</p>
        <Button variant="primary" onClick={onDone}>
          Concluir
        </Button>
      </div>
    );
  }

  const remaining = pairing ? pairing.expiresAt - now : 0;
  const progress = Math.max(0, Math.min(1, remaining / PAIRING_WINDOW_MS));

  return (
    <div className="pairing">
      <ol className="steps">
        <li>
          <span>
            No Android, abra o PC Remote, vá em <strong>Conexão</strong> e toque em <strong>Ler QR Code</strong>.
          </span>
        </li>
        <li>
          <span>Aponte a câmera para o código abaixo.</span>
        </li>
        <li>
          <span>Confirme o pedido que vai aparecer aqui.</span>
        </li>
      </ol>

      <div className="qr-stage">
        <div className={active ? 'qr-frame' : 'qr-frame is-expired'}>
          {qr && active ? (
            <img src={qr} alt="QR Code com o endereço, a identidade deste PC e o código temporário" />
          ) : generating ? (
            <LoaderCircle className="spin" aria-label="Gerando código" />
          ) : (
            <Clock aria-hidden />
          )}
        </div>
        <div className="pair-code-block">
          <span className="field-label">Código de pareamento</span>
          <span className="pair-code tabular" aria-label={pairing ? `Código ${pairing.code.split('').join(' ')}` : undefined}>
            {pairing && active ? groupCode(pairing.code) : '•••• ••••'}
          </span>
          {active ? (
            <span className="countdown" aria-live="off">
              <span className="countdown-bar" aria-hidden>
                <span style={{ transform: `scaleX(${progress})` }} />
              </span>
              <span className="tabular">Expira em {formatCountdown(remaining)}</span>
            </span>
          ) : (
            <span className="countdown is-expired">{generating ? 'Gerando um novo código…' : 'Código expirado'}</span>
          )}
        </div>
      </div>

      {status.pending.length > 0 ? (
        <section className="pair-requests" aria-label="Pedidos recebidos">
          {status.pending.map(request => (
            <PendingRequest
              key={request.id}
              request={request}
              now={now}
              deciding={isBusy(`decide:${request.id}`)}
              onDecide={approved => decide(request.id, request.deviceName, approved)}
            />
          ))}
        </section>
      ) : active ? (
        <p className="waiting-line" role="status">
          <span className="waiting-dot" aria-hidden />
          Aguardando o celular ler o código
        </p>
      ) : null}

      <div className="pair-actions">
        <Button
          icon={RefreshCw}
          loading={generating}
          disabled={status.pending.length > 0}
          title={status.pending.length > 0 ? 'Responda o pedido pendente antes de gerar outro código.' : undefined}
          onClick={() => void run('pair', desktopApi.startPairing)}
        >
          {active ? 'Gerar novo código' : 'Gerar código'}
        </Button>
      </div>

      <details className="manual">
        <summary>Digitar os dados no celular</summary>
        <dl className="manual-list">
          <div>
            <dt>Endereço</dt>
            <dd>
              <span className="mono">{address ?? 'Indisponível'}</span>
              {address ? <CopyButton value={address} label="endereço" /> : null}
            </dd>
          </div>
          <div>
            <dt>Porta</dt>
            <dd>
              <span className="mono tabular">{status.port}</span>
              <CopyButton value={String(status.port)} label="porta" />
            </dd>
          </div>
          <div>
            <dt>Código</dt>
            <dd>
              <span className="mono tabular">{pairing && active ? pairing.code : '—'}</span>
              {pairing && active ? <CopyButton value={pairing.code} label="código" /> : null}
            </dd>
          </div>
          <div className="manual-fingerprint">
            <dt>Identidade SHA-256</dt>
            <dd>
              <code className="mono">{status.fingerprint}</code>
              <span className="manual-fp-row">
                <FingerprintStrip value={status.fingerprint} size="sm" />
                <CopyButton value={status.fingerprint} label="identidade SHA-256" />
              </span>
            </dd>
          </div>
        </dl>
      </details>
    </div>
  );
}
