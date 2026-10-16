import { useState } from 'react';
import { QrCode, ShieldCheck, Smartphone } from 'lucide-react';
import { desktopApi } from '../api';
import { Button } from '../components/Button';
import { ConfirmDialog } from '../components/ConfirmDialog';
import { DeviceRow, PendingRequest } from '../components/DeviceItems';
import { EmptyState } from '../components/EmptyState';
import { plural } from '../lib/format';
import type { Device } from '../types';
import type { ViewProps } from './viewProps';

export function DevicesView({ status, run, isBusy, now, openPairing }: ViewProps) {
  const [revoking, setRevoking] = useState<Device | null>(null);
  const online = status.devices.filter(device => device.connected).length;
  const devices = [...status.devices].sort((a, b) => Number(b.connected) - Number(a.connected) || a.name.localeCompare(b.name, 'pt-BR'));

  const confirmRevoke = () => {
    if (!revoking) return;
    const device = revoking;
    void run(`revoke:${device.id}`, () => desktopApi.revokeDevice(device.id), `Acesso de ${device.name} revogado.`).then(ok => {
      // On failure the dialog stays open so the user can retry right away.
      if (ok) setRevoking(null);
    });
  };

  return (
    <div className="view">
      <div className="view-intro">
        <p>Somente celulares aprovados aqui podem controlar este PC, e um de cada vez.</p>
        <Button variant="primary" icon={QrCode} onClick={openPairing}>
          Parear celular
        </Button>
      </div>

      {status.pending.length > 0 ? (
        <section className="card card-attention" aria-labelledby="pending-title">
          <header className="card-header">
            <h2 id="pending-title" className="card-title">
              Aguardando sua confirmação
            </h2>
            <span className="badge badge-warning">{plural(status.pending.length, 'pedido', 'pedidos')}</span>
          </header>
          <div className="stack-sm">
            {status.pending.map(request => (
              <PendingRequest
                key={request.id}
                request={request}
                now={now}
                deciding={isBusy(`decide:${request.id}`)}
                onDecide={approved =>
                  void run(
                    `decide:${request.id}`,
                    () => desktopApi.approvePairing(request.id, approved),
                    approved ? `${request.deviceName} foi autorizado.` : 'Pedido recusado.',
                  )
                }
              />
            ))}
          </div>
        </section>
      ) : null}

      <section className="card" aria-labelledby="authorized-title">
        <header className="card-header">
          <h2 id="authorized-title" className="card-title">
            Celulares autorizados
          </h2>
          {status.devices.length > 0 ? (
            <span className="card-meta tabular">
              {plural(status.devices.length, 'pareado', 'pareados')}, {online} online
            </span>
          ) : null}
        </header>
        {devices.length > 0 ? (
          <ul className="device-list">
            {devices.map(device => (
              <DeviceRow
                key={device.id}
                device={device}
                now={now}
                action={
                  <Button variant="danger-ghost" size="sm" loading={isBusy(`revoke:${device.id}`)} onClick={() => setRevoking(device)}>
                    Revogar acesso
                  </Button>
                }
              />
            ))}
          </ul>
        ) : (
          <EmptyState icon={Smartphone} title="Nenhum celular autorizado">
            Quando você aprovar um pedido de pareamento, o celular aparece aqui com o status da conexão.
          </EmptyState>
        )}
        <p className="card-footnote">
          <ShieldCheck aria-hidden />O horário de última conexão fica só na memória e é zerado quando o PC Remote é encerrado.
        </p>
      </section>

      <ConfirmDialog
        open={revoking !== null}
        title="Revogar acesso?"
        confirmLabel="Revogar acesso"
        busy={revoking ? isBusy(`revoke:${revoking.id}`) : false}
        onConfirm={confirmRevoke}
        onClose={() => setRevoking(null)}
      >
        <p>
          <strong>{revoking?.name}</strong> será desconectado agora e não poderá mais controlar este PC. Para usá-lo de novo, será
          preciso parear outra vez.
        </p>
      </ConfirmDialog>
    </div>
  );
}
