import type { ReactNode } from 'react';
import { ShieldCheck, ShieldX, Smartphone } from 'lucide-react';
import { formatDay, formatRelative } from '../lib/format';
import { cx } from '../lib/cx';
import type { Device, PendingDevice } from '../types';
import { Button } from './Button';

export function deviceActivity(device: Device, now: number): string {
  if (device.connected) return 'Conectado agora';
  if (device.lastSeenAt) return `Última conexão ${formatDay(device.lastSeenAt, now)}`;
  return 'Sem conexão desde que o PC Remote foi aberto';
}

export function DeviceRow({ device, now, action }: { device: Device; now: number; action?: ReactNode }) {
  return (
    <li className="device-row">
      <span className={cx('device-avatar', device.connected && 'is-online')} aria-hidden>
        <Smartphone />
      </span>
      <div className="device-text">
        <p className="device-name">{device.name}</p>
        <p className="device-meta">{deviceActivity(device, now)}</p>
      </div>
      <span className={cx('badge', device.connected ? 'badge-success' : 'badge-neutral')}>{device.connected ? 'Online' : 'Offline'}</span>
      {action ? <div className="device-action">{action}</div> : null}
    </li>
  );
}

interface PendingRequestProps {
  request: PendingDevice;
  now: number;
  deciding: boolean;
  onDecide: (approved: boolean) => void;
}

/** Access request preview with the decision right next to it. */
export function PendingRequest({ request, now, deciding, onDecide }: PendingRequestProps) {
  return (
    <div className="request" role="group" aria-label={`Pedido de acesso de ${request.deviceName}`}>
      <span className="request-avatar" aria-hidden>
        <Smartphone />
      </span>
      <div className="request-text">
        <p className="request-title">{request.deviceName}</p>
        <p className="request-meta">
          Pediu acesso {formatRelative(request.requestedAt, now)}. Permita apenas se for o celular que você está pareando agora.
        </p>
      </div>
      <div className="request-actions">
        <Button variant="danger-ghost" icon={ShieldX} disabled={deciding} onClick={() => onDecide(false)}>
          Recusar
        </Button>
        <Button variant="primary" icon={ShieldCheck} loading={deciding} onClick={() => onDecide(true)}>
          Permitir
        </Button>
      </div>
    </div>
  );
}
