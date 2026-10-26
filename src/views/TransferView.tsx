import { ArrowDownLeft, ArrowLeft, ArrowRight, ArrowUpRight, ClipboardList, FileText, FolderOpen, Hand, History, LockKeyholeOpen, Monitor, Smartphone, Upload, X } from 'lucide-react';
import { desktopApi } from '../api';
import { Button } from '../components/Button';
import { SettingSwitch } from '../components/Switch';
import { useToast } from '../hooks/useToasts';
import { cx } from '../lib/cx';
import { formatBytes, formatClock, plural } from '../lib/format';
import { errorMessage, type DesktopStatus, type FileEntry, type Settings, type TransferEntry } from '../types';
import { clipboardSummary, fileProgress, inputSummary, sendSummary, transferMeta } from './summaries';
import type { RunAction, ViewProps } from './viewProps';

function SyncFlow({ settings }: { settings: Settings }) {
  const on = settings.realtime;
  const label = `Sincronização em tempo real ${on ? 'ligada' : 'desligada'}.`;
  return (
    <div className={cx('sync-flow', !on && 'is-paused')} role="img" aria-label={label}>
      <span className="flow-node">
        <Monitor aria-hidden />
        <span>Este PC</span>
      </span>
      <span className="flow-lanes" aria-hidden>
        <span className={cx('flow-lane', on && 'is-on')}>
          <span className="flow-line" />
          <ArrowRight />
        </span>
        <span className={cx('flow-lane', on && 'is-on')}>
          <ArrowLeft />
          <span className="flow-line" />
        </span>
      </span>
      <span className="flow-node">
        <Smartphone aria-hidden />
        <span>Celular</span>
      </span>
    </div>
  );
}

function TransferRow({ entry }: { entry: TransferEntry }) {
  const toPhone = entry.direction === 'toPhone';
  const Icon = toPhone ? ArrowUpRight : ArrowDownLeft;
  return (
    <li className="transfer-row">
      <span className={cx('transfer-icon', toPhone ? 'is-out' : 'is-in')} aria-hidden>
        <Icon />
      </span>
      <span className="transfer-text">
        <span className="transfer-title">{toPhone ? 'Enviado ao celular' : 'Recebido do celular'}</span>
        <span className="transfer-meta tabular">{transferMeta(entry)}</span>
      </span>
      <time className="transfer-time tabular" dateTime={new Date(entry.at).toISOString()}>
        {formatClock(entry.at, true)}
      </time>
    </li>
  );
}

function FileRow({ file }: { file: FileEntry }) {
  const toPhone = file.direction === 'toPhone';
  return (
    <li className="transfer-row">
      <span className={cx('transfer-icon', toPhone ? 'is-out' : 'is-in')} aria-hidden>
        {toPhone ? <ArrowUpRight /> : <FileText />}
      </span>
      <span className="transfer-text">
        <span className="transfer-title is-file" title={file.name}>
          {file.name}
        </span>
        <span className="transfer-meta tabular">
          {toPhone ? 'Enviado ao celular' : 'Recebido do celular'}, {formatBytes(file.bytes) || '0 B'}
        </span>
      </span>
      <time className="transfer-time tabular" dateTime={new Date(file.at).toISOString()}>
        {formatClock(file.at, true)}
      </time>
    </li>
  );
}

function ProgressRow({ label, name, done, total, waiting, onCancel }: { label: string; name: string; done: number; total: number; waiting?: boolean; onCancel?: () => void }) {
  const progress = fileProgress({ bytes: total, received: done });
  return (
    <div className="incoming-file">
      <div className="incoming-head">
        <span className="incoming-name" title={name}>
          {name}
        </span>
        <span className="incoming-meta tabular">
          {waiting ? 'Aguardando o celular…' : `${formatBytes(done) || '0 B'} de ${formatBytes(total) || '0 B'}`}
        </span>
        {onCancel ? (
          <Button variant="ghost" size="sm" icon={X} onClick={onCancel}>
            Cancelar
          </Button>
        ) : null}
      </div>
      <div className="progress-track" role="progressbar" aria-label={`${label} ${name}`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(progress * 100)}>
        <span style={{ transform: `scaleX(${progress})` }} />
      </div>
    </div>
  );
}

/** "Enviar ao celular": opens the native picker; the backend keeps the paths and the queue. */
export function useSendFiles(run: RunAction) {
  const toast = useToast();
  return () =>
    void run('sendFiles', async () => {
      const problems = await desktopApi.sendFilesDialog();
      problems.forEach(problem => toast.error(problem));
    });
}

export function InputControls({ status, settings, run, isBusy }: Pick<ViewProps, 'status' | 'settings' | 'run' | 'isBusy'>) {
  if (settings.inputBlocked) {
    return (
      <div className="kill-switch">
        <div>
          <p className="kill-title">Mouse, teclado e energia bloqueados</p>
          <p className="muted">Libere para retomar o controle pelo celular.</p>
        </div>
        <Button variant="primary" icon={LockKeyholeOpen} loading={isBusy('unblockInput')} onClick={() => void run('unblockInput', desktopApi.unblockInput, 'Controle pelo celular liberado.')}>
          Liberar
        </Button>
      </div>
    );
  }
  return (
    <div className="kill-switch">
      <div>
        <p className="kill-title">{status.inputActive ? 'Interromper controle' : 'Bloquear controle pelo celular'}</p>
        <p className="muted">Pausa mouse, teclado e ações de energia.</p>
      </div>
      <Button variant={status.inputActive ? 'danger' : 'secondary'} icon={Hand} loading={isBusy('stopInput')} onClick={() => void run('stopInput', desktopApi.stopInput, 'Mouse, teclado e energia bloqueados.')}>
        {status.inputActive ? 'Interromper' : 'Bloquear'}
      </Button>
    </div>
  );
}

function OutgoingArea({ status, run, isBusy }: { status: DesktopStatus; run: RunAction; isBusy: (key: string) => boolean }) {
  const toast = useToast();
  const sendFiles = useSendFiles(run);
  const { outgoing, outgoingQueue: queue, outgoingResult: result } = status;
  const attempt = (action: () => Promise<unknown>) => action().catch(reason => toast.error(errorMessage(reason)));
  return (
    <div className="stack-sm">
      <div className="kill-switch">
        <div>
          <p className="kill-title">Enviar ao celular</p>
          <p className="muted">{sendSummary(status)}</p>
        </div>
        <Button variant="primary" icon={Upload} loading={isBusy('sendFiles')} onClick={sendFiles}>
          Escolher arquivos
        </Button>
      </div>
      {outgoing ? (
        <ProgressRow label="Enviando" name={outgoing.name} done={outgoing.sent} total={outgoing.bytes} waiting={outgoing.waiting} onCancel={() => void attempt(desktopApi.cancelOutgoing)} />
      ) : null}
      {queue.length > 0 ? (
        <div className="subsection-header">
          <p className="muted">{outgoing ? `Mais ${plural(queue.length, 'arquivo', 'arquivos')} na fila.` : `${plural(queue.length, 'arquivo', 'arquivos')} esperando o celular.`}</p>
          <Button variant="ghost" size="sm" onClick={() => void attempt(desktopApi.clearOutgoing)}>
            Limpar fila
          </Button>
        </div>
      ) : null}
      {result && !outgoing ? (
        <p className={cx('muted', !result.ok && 'text-danger')} role="status">
          {result.message}
        </p>
      ) : null}
    </div>
  );
}

export function TransferView({ status, settings, changeSettings, run, isBusy }: ViewProps) {
  return (
    <div className="view">
      <section className="card" aria-labelledby="files-title">
        <header className="card-header">
          <h2 id="files-title" className="card-title">
            Arquivos
          </h2>
          <Button variant="ghost" size="sm" icon={FolderOpen} loading={isBusy('openFolder')} onClick={() => void run('openFolder', desktopApi.openReceivedFolder)}>
            Abrir pasta
          </Button>
        </header>
        <OutgoingArea status={status} run={run} isBusy={isBusy} />
        {status.incomingFile ? <ProgressRow label="Recebendo" name={status.incomingFile.name} done={status.incomingFile.received} total={status.incomingFile.bytes} /> : null}
        <div className="subsection">
          <h3 className="subsection-title">
            <History aria-hidden />
            Últimos arquivos
          </h3>
          {status.fileHistory.length > 0 ? (
            <ul className="transfer-list">
              {status.fileHistory.map((file, index) => (
                <FileRow key={`${index}-${file.at}`} file={file} />
              ))}
            </ul>
          ) : (
            <p className="muted subsection-empty">Nenhum arquivo ainda.</p>
          )}
        </div>
      </section>

      <section className="card" aria-labelledby="clipboard-title">
        <header className="card-header">
          <h2 id="clipboard-title" className="card-title">
            Área de transferência
          </h2>
        </header>
        <SyncFlow settings={settings} />
        <div className="setting-list">
          <SettingSwitch
            icon={ClipboardList}
            label="Sincronização em tempo real"
            description={clipboardSummary(settings)}
            checked={settings.realtime}
            onChange={realtime => changeSettings({ realtime })}
          />
        </div>
        <div className="subsection">
          <h3 className="subsection-title">
            <History aria-hidden />
            Últimas cópias
          </h3>
          {status.transfers.length > 0 ? (
            <ul className="transfer-list">
              {status.transfers.map((entry, index) => (
                <TransferRow key={`${index}-${entry.at}`} entry={entry} />
              ))}
            </ul>
          ) : (
            <p className="muted subsection-empty">Nenhuma cópia ainda.</p>
          )}
        </div>
      </section>

      <section className="card" aria-labelledby="input-title">
        <header className="card-header">
          <h2 id="input-title" className="card-title">
            Mouse e teclado
          </h2>
          <span className={cx('badge', settings.inputBlocked ? 'badge-neutral' : status.inputActive ? 'badge-success' : 'badge-accent')}>{inputSummary(settings, status.inputActive)}</span>
        </header>
        <InputControls status={status} settings={settings} run={run} isBusy={isBusy} />
      </section>
    </div>
  );
}
