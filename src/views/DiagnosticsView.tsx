import { ClipboardCopy, LockKeyhole, RefreshCw, ShieldCheck } from 'lucide-react';
import { useState } from 'react';
import { Button } from '../components/Button';
import { CopyButton } from '../components/CopyButton';
import { EmptyState } from '../components/EmptyState';
import { FingerprintPanel } from '../components/Fingerprint';
import { NetworkAccessPanel } from '../components/NetworkAccessPanel';
import { useToast } from '../hooks/useToasts';
import { copyText } from '../lib/clipboard';
import { formatClock, plural } from '../lib/format';
import { powerSummary } from './summaries';
import { errorMessage, PROTOCOL_VERSION, preferredAddress, type DesktopStatus, type NetworkAccessStatus } from '../types';
import type { ViewProps } from './viewProps';

interface DiagnosticsViewProps extends ViewProps {
  appVersion: string;
  refresh: () => Promise<void>;
}

/** Plain-text report for support. Contains no tokens, clipboard text or typed input. */
export function buildReport(status: DesktopStatus, appVersion: string, serviceLabel: string, networkAccess?: NetworkAccessStatus | null): string {
  const lines = [
    'PC Remote — relatório de diagnóstico',
    `Gerado em: ${new Date().toISOString()}`,
    `Versão do aplicativo: ${appVersion}`,
    `Protocolo: ${PROTOCOL_VERSION}`,
    `Serviço: ${serviceLabel}`,
    `Computador: ${status.pcName}`,
    `Endereços: ${status.addresses.join(', ') || 'nenhum'}`,
    `Porta: ${status.port}`,
    ...(networkAccess?.supported ? [
      `Permissões de rede: ${networkAccess.state}`,
      `Perfil de rede: ${networkAccess.profile || 'indisponível'}`,
      `Verificação de rede: ${networkAccess.message}`,
    ] : []),
    `Identidade SHA-256: ${status.fingerprint || 'indisponível'}`,
    `Celulares pareados: ${status.devices.length} (${status.devices.filter(device => device.connected).length} conectado)`,
    `Pedidos pendentes: ${status.pending.length}`,
    `Sincronização em tempo real: ${status.settings.realtime ? 'ligada' : 'desligada'}`,
    `Mouse, teclado e energia: ${status.settings.inputBlocked ? 'bloqueado' : 'liberado'}${status.inputActive ? ', em uso' : ''}`,
    `Energia agendada: ${status.powerScheduled ? powerSummary(status.powerScheduled, Date.now()) : 'nada'}`,
    `Arquivos: ${plural(status.fileHistory.length, 'transferido', 'transferidos')} nesta execução${status.incomingFile ? ', recebendo' : ''}${status.outgoing ? ', enviando' : ''}, ${plural(status.outgoingQueue.length, 'na fila', 'na fila')}; celular ${status.phoneReceivesFiles ? 'recebe' : 'não recebe'} arquivos do PC`,
    `Iniciar com o Windows: ${status.settings.autostart ? 'sim' : 'não'}`,
    '',
    'Eventos recentes:',
    ...(status.diagnostics.length
      ? status.diagnostics.map(entry => `${new Date(entry.at).toISOString()}  ${entry.message}`)
      : ['(nenhum)']),
  ];
  return lines.join('\n');
}

export function DiagnosticsView({ status, serviceState, appVersion, refresh }: DiagnosticsViewProps) {
  const toast = useToast();
  const [networkAccess, setNetworkAccess] = useState<NetworkAccessStatus | null>(null);
  const address = preferredAddress(status.addresses);
  const serviceLabel = serviceState === 'unavailable' ? 'Indisponível' : status.serviceRunning ? 'Em execução' : 'Parado';
  const events = [...status.diagnostics].reverse();

  const copyReport = () => {
    copyText(buildReport(status, appVersion, serviceLabel, networkAccess))
      .then(() => toast.success('Relatório copiado.'))
      .catch(reason => toast.error(errorMessage(reason)));
  };

  return (
    <div className="view">
      <section className="card" aria-labelledby="identity-title">
        <header className="card-header">
          <div>
            <h2 id="identity-title" className="card-title">
              Identidade deste PC
            </h2>
            <p className="card-description">Ao parear, os blocos de código devem ser iguais aos do celular.</p>
          </div>
          <ShieldCheck className="card-glyph" aria-hidden />
        </header>
        <FingerprintPanel value={status.fingerprint} />
      </section>

      <section className="card" aria-labelledby="details-title">
        <header className="card-header">
          <h2 id="details-title" className="card-title">
            Conexão local
          </h2>
          <Button variant="ghost" size="sm" icon={RefreshCw} onClick={() => void refresh()}>
            Atualizar
          </Button>
        </header>
        <dl className="detail-list">
          <div>
            <dt>Serviço</dt>
            <dd>
              <span className={serviceState === 'unavailable' ? 'text-danger' : 'text-success'}>{serviceLabel}</span>
            </dd>
          </div>
          <div>
            <dt>Computador</dt>
            <dd>{status.pcName}</dd>
          </div>
          <div>
            <dt>Endereço principal</dt>
            <dd>
              <span className="mono">{address ? `${address}:${status.port}` : 'Sem rede local'}</span>
              {address ? <CopyButton value={`${address}:${status.port}`} label="endereço principal" /> : null}
            </dd>
          </div>
          {status.addresses.length > 1 ? (
            <div>
              <dt>Outros endereços</dt>
              <dd className="mono">{status.addresses.filter(item => item !== address).join(', ')}</dd>
            </div>
          ) : null}
          <div>
            <dt>ID do PC</dt>
            <dd>
              <span className="mono">{status.pcId}</span>
              <CopyButton value={status.pcId} label="ID do PC" />
            </dd>
          </div>
          <div>
            <dt>Versão</dt>
            <dd className="tabular">
              {appVersion}, protocolo {PROTOCOL_VERSION}
            </dd>
          </div>
        </dl>
      </section>

      <NetworkAccessPanel card onStatus={setNetworkAccess} />

      <section className="card" aria-labelledby="events-title">
        <header className="card-header">
          <h2 id="events-title" className="card-title">
            Eventos recentes
          </h2>
          <Button icon={ClipboardCopy} onClick={copyReport}>
            Copiar relatório
          </Button>
        </header>
        {events.length > 0 ? (
          <ol className="event-log" aria-label="Registro de eventos">
            {events.map((entry, index) => (
              <li key={`${entry.at}-${index}`}>
                <time className="tabular" dateTime={new Date(entry.at).toISOString()}>
                  {formatClock(entry.at, true)}
                </time>
                <span>{entry.message}</span>
              </li>
            ))}
          </ol>
        ) : (
          <EmptyState icon={ShieldCheck} title="Nenhum evento" />
        )}
        <p className="card-footnote">
          <LockKeyhole aria-hidden />
          O relatório não inclui textos copiados nem credenciais.
        </p>
      </section>
    </div>
  );
}
