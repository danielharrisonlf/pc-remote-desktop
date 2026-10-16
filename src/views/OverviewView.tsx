import { ArrowRight, ClipboardList, Monitor, QrCode, Smartphone, Upload } from 'lucide-react';
import { desktopApi } from '../api';
import { Button } from '../components/Button';
import { DeviceRow, PendingRequest } from '../components/DeviceItems';
import { EmptyState } from '../components/EmptyState';
import { SettingSwitch } from '../components/Switch';
import { clipboardSummary, sendSummary } from './summaries';
import { InputControls, useSendFiles } from './TransferView';
import type { ViewProps } from './viewProps';

const DEVICE_PREVIEW = 3;

export function OverviewView({ status, settings, changeSettings, run, isBusy, serviceState, now, navigate, openPairing }: ViewProps) {
  const sendFiles = useSendFiles(run);
  const connectedDevice = status.devices.find(device => device.connected);
  const subtitle =
    serviceState === 'unavailable'
      ? 'O serviço não está respondendo.'
      : connectedDevice
        ? `Conectado a ${connectedDevice.name}`
        : 'Aguardando conexão';
  const devices = [...status.devices].sort((a, b) => Number(b.connected) - Number(a.connected) || (b.lastSeenAt ?? 0) - (a.lastSeenAt ?? 0));

  return (
    <div className="view">
      <section className="card hero" aria-labelledby="hero-title">
        <div className="hero-main">
          <span className="hero-icon" aria-hidden>
            <Monitor />
          </span>
          <div className="hero-text">
            <h2 id="hero-title" className="hero-title">
              {status.pcName}
            </h2>
            <p className="hero-subtitle">{subtitle}</p>
          </div>
          <Button variant="primary" size="lg" icon={QrCode} onClick={openPairing}>
            Parear celular
          </Button>
        </div>
      </section>

      <div className="overview-grid">
        <section className="card" aria-labelledby="overview-devices">
          <header className="card-header">
            <h2 id="overview-devices" className="card-title">
              Celulares
            </h2>
            {status.devices.length > 0 ? (
              <Button variant="ghost" size="sm" onClick={() => navigate('devices')}>
                Gerenciar
                <ArrowRight className="btn-trailing" aria-hidden />
              </Button>
            ) : null}
          </header>
          {status.pending.length > 0 ? (
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
          ) : null}
          {devices.length > 0 ? (
            <ul className="device-list">
              {devices.slice(0, DEVICE_PREVIEW).map(device => (
                <DeviceRow key={device.id} device={device} now={now} />
              ))}
            </ul>
          ) : status.pending.length === 0 ? (
            <EmptyState
              icon={Smartphone}
              title="Nenhum celular pareado"
              action={
                <Button variant="secondary" icon={QrCode} onClick={openPairing}>
                  Parear celular
                </Button>
              }
            />
          ) : null}
          {devices.length > DEVICE_PREVIEW ? (
            <p className="card-footnote">
              Mais {devices.length - DEVICE_PREVIEW} em <button type="button" className="inline-link" onClick={() => navigate('devices')}>Dispositivos</button>.
            </p>
          ) : null}
        </section>

        <section className="card" aria-labelledby="overview-controls">
          <header className="card-header">
            <h2 id="overview-controls" className="card-title">
              Controles rápidos
            </h2>
          </header>
          <div className="setting-list">
            <SettingSwitch
              icon={ClipboardList}
              label="Sincronização em tempo real"
              description={clipboardSummary(settings)}
              checked={settings.realtime}
              onChange={realtime => changeSettings({ realtime })}
            />
          </div>
          <Button variant="secondary" icon={Upload} className="block-btn" loading={isBusy('sendFiles')} onClick={sendFiles}>
            Enviar arquivos ao celular
          </Button>
          <p className="card-footnote">{status.outgoing ? `Enviando ${status.outgoing.name}…` : sendSummary(status)}</p>
          {status.inputActive || settings.inputBlocked ? <InputControls status={status} settings={settings} run={run} isBusy={isBusy} /> : null}
          <button type="button" className="card-link" onClick={() => navigate('transfer')}>
            Arquivos e histórico
            <ArrowRight aria-hidden />
          </button>
        </section>
      </div>
    </div>
  );
}
