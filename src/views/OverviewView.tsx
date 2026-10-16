import { ArrowRight, ClipboardList, FileImage, Hand, Monitor, MousePointer2, QrCode, Smartphone } from 'lucide-react';
import { desktopApi } from '../api';
import { Button } from '../components/Button';
import { CopyButton } from '../components/CopyButton';
import { DeviceRow, PendingRequest } from '../components/DeviceItems';
import { EmptyState } from '../components/EmptyState';
import { FingerprintInline } from '../components/Fingerprint';
import { SettingSwitch } from '../components/Switch';
import { preferredAddress } from '../types';
import { clipboardSummary, filesSummary, inputSummary } from './summaries';
import type { ViewProps } from './viewProps';

const DEVICE_PREVIEW = 3;

export function OverviewView({ status, settings, changeSettings, run, isBusy, serviceState, now, navigate, openPairing }: ViewProps) {
  const address = preferredAddress(status.addresses);
  const endpoint = address ? `${address}:${status.port}` : null;
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
        <dl className="hero-facts">
          <div className="fact">
            <dt>Endereço</dt>
            <dd>
              {endpoint ? (
                <>
                  <span className="fact-value mono">{endpoint}</span>
                  <CopyButton value={endpoint} label="endereço e porta" />
                </>
              ) : (
                <span className="fact-value muted">Sem rede</span>
              )}
            </dd>
          </div>
          <div className="fact">
            <dt>Identidade</dt>
            <dd>
              <FingerprintInline value={status.fingerprint} />
            </dd>
          </div>
          <div className="fact">
            <dt>Celular conectado</dt>
            <dd>
              <span className="fact-value">{connectedDevice ? connectedDevice.name : 'Nenhum'}</span>
            </dd>
          </div>
        </dl>
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
              label="Sincronizar área de transferência"
              description={clipboardSummary(settings)}
              checked={!settings.paused}
              onChange={enabled => changeSettings({ paused: !enabled })}
            />
            <SettingSwitch
              icon={MousePointer2}
              label="Mouse e teclado pelo celular"
              description={inputSummary(settings, status.inputActive)}
              checked={settings.allowInput}
              onChange={allowInput => changeSettings({ allowInput })}
            />
            <SettingSwitch
              icon={FileImage}
              label="Imagens e PDFs do celular"
              description={filesSummary(settings, status.incomingFile)}
              checked={settings.allowFiles}
              onChange={allowFiles => changeSettings({ allowFiles })}
            />
          </div>
          {status.inputActive ? (
            <Button
              variant="danger"
              icon={Hand}
              className="block-btn"
              loading={isBusy('stopInput')}
              onClick={() => void run('stopInput', desktopApi.stopInput, 'Controle interrompido.')}
            >
              Interromper controle
            </Button>
          ) : null}
          <button type="button" className="card-link" onClick={() => navigate('transfer')}>
            Mais opções
            <ArrowRight aria-hidden />
          </button>
        </section>
      </div>
    </div>
  );
}
