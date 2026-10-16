import { ArrowRight, ClipboardList, Hand, Monitor, MousePointer2, QrCode, Smartphone } from 'lucide-react';
import { desktopApi } from '../api';
import { Button } from '../components/Button';
import { CopyButton } from '../components/CopyButton';
import { DeviceRow, PendingRequest } from '../components/DeviceItems';
import { EmptyState } from '../components/EmptyState';
import { FingerprintInline } from '../components/Fingerprint';
import { SettingSwitch } from '../components/Switch';
import { preferredAddress } from '../types';
import { clipboardSummary, inputSummary } from './summaries';
import type { ViewProps } from './viewProps';

const DEVICE_PREVIEW = 3;

export function OverviewView({ status, settings, changeSettings, run, isBusy, serviceState, now, navigate, openPairing }: ViewProps) {
  const address = preferredAddress(status.addresses);
  const endpoint = address ? `${address}:${status.port}` : null;
  const connectedDevice = status.devices.find(device => device.connected);
  const subtitle =
    serviceState === 'unavailable'
      ? 'O serviço local não respondeu. Os dados abaixo podem estar desatualizados.'
      : connectedDevice
        ? `Controlado por ${connectedDevice.name} neste momento.`
        : 'Pronto para receber celulares pareados nesta rede.';
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
            <dt>Endereço na rede local</dt>
            <dd>
              {endpoint ? (
                <>
                  <span className="fact-value mono">{endpoint}</span>
                  <CopyButton value={endpoint} label="endereço e porta" />
                </>
              ) : (
                <span className="fact-value muted">Sem rede local</span>
              )}
            </dd>
          </div>
          <div className="fact">
            <dt>Identidade do PC</dt>
            <dd>
              <FingerprintInline value={status.fingerprint} />
            </dd>
          </div>
          <div className="fact">
            <dt>Sessão</dt>
            <dd>
              <span className="fact-value">{connectedDevice ? connectedDevice.name : 'Nenhum celular conectado'}</span>
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
                  Parear o primeiro celular
                </Button>
              }
            >
              Pareie o celular para controlar mídia, volume, mouse e teclado deste PC.
            </EmptyState>
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
          </div>
          {status.inputActive ? (
            <Button
              variant="danger"
              icon={Hand}
              className="block-btn"
              loading={isBusy('stopInput')}
              onClick={() => void run('stopInput', desktopApi.stopInput, 'Controle remoto interrompido. Ative de novo no celular quando precisar.')}
            >
              Interromper controle agora
            </Button>
          ) : null}
          <button type="button" className="card-link" onClick={() => navigate('transfer')}>
            Direções, histórico e detalhes
            <ArrowRight aria-hidden />
          </button>
        </section>
      </div>
    </div>
  );
}
