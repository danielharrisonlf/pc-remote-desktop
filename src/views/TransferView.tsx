import { ArrowDownLeft, ArrowLeft, ArrowRight, ArrowUpRight, FileImage, FileText, FolderOpen, Hand, History, Monitor, MousePointer2, Pause, Play, Smartphone } from 'lucide-react';
import { desktopApi } from '../api';
import { Button } from '../components/Button';
import { SettingSwitch } from '../components/Switch';
import { cx } from '../lib/cx';
import { formatBytes, formatClock, plural } from '../lib/format';
import type { IncomingFile, ReceivedFile, Settings, TransferEntry } from '../types';
import { clipboardSummary, fileProgress, filesSummary } from './summaries';
import type { ViewProps } from './viewProps';

function SyncFlow({ settings }: { settings: Settings }) {
  const toPhone = settings.upload && !settings.paused;
  const toPc = settings.download && !settings.paused;
  const label = `Do PC para o celular ${toPhone ? 'ligado' : 'desligado'}; do celular para o PC ${toPc ? 'ligado' : 'desligado'}.`;
  return (
    <div className={cx('sync-flow', settings.paused && 'is-paused')} role="img" aria-label={label}>
      <span className="flow-node">
        <Monitor aria-hidden />
        <span>Este PC</span>
      </span>
      <span className="flow-lanes" aria-hidden>
        <span className={cx('flow-lane', toPhone && 'is-on')}>
          <span className="flow-line" />
          <ArrowRight />
        </span>
        <span className={cx('flow-lane', toPc && 'is-on')}>
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
        <span className="transfer-meta tabular">
          {plural(entry.characters, 'caractere', 'caracteres')}, {entry.automatic ? 'automático' : 'manual'}
        </span>
      </span>
      <time className="transfer-time tabular" dateTime={new Date(entry.at).toISOString()}>
        {formatClock(entry.at, true)}
      </time>
    </li>
  );
}

function ReceivedRow({ file }: { file: ReceivedFile }) {
  const Icon = /\.pdf$/i.test(file.name) ? FileText : FileImage;
  return (
    <li className="transfer-row">
      <span className="transfer-icon is-in" aria-hidden>
        <Icon />
      </span>
      <span className="transfer-text">
        <span className="transfer-title is-file" title={file.name}>
          {file.name}
        </span>
        <span className="transfer-meta tabular">{formatBytes(file.bytes)}</span>
      </span>
      <time className="transfer-time tabular" dateTime={new Date(file.at).toISOString()}>
        {formatClock(file.at, true)}
      </time>
    </li>
  );
}

function IncomingRow({ file }: { file: IncomingFile }) {
  const progress = fileProgress(file);
  return (
    <div className="incoming-file">
      <div className="incoming-head">
        <span className="incoming-name" title={file.name}>
          {file.name}
        </span>
        <span className="incoming-meta tabular">
          {formatBytes(file.received) || '0 B'} de {formatBytes(file.bytes)}
        </span>
      </div>
      <div
        className="progress-track"
        role="progressbar"
        aria-label={`Recebendo ${file.name}`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(progress * 100)}
      >
        <span style={{ transform: `scaleX(${progress})` }} />
      </div>
    </div>
  );
}

export function TransferView({ status, settings, changeSettings, run, isBusy }: ViewProps) {
  const inputBadge = status.inputActive
    ? { tone: 'badge-success', text: 'Em uso' }
    : settings.allowInput
      ? { tone: 'badge-accent', text: 'Liberado' }
      : { tone: 'badge-neutral', text: 'Bloqueado' };
  const filesTone = status.incomingFile ? 'badge-success' : settings.allowFiles ? 'badge-accent' : 'badge-neutral';

  return (
    <div className="view">
      <section className="card" aria-labelledby="clipboard-title">
        <header className="card-header">
          <div>
            <h2 id="clipboard-title" className="card-title">
              Área de transferência
            </h2>
            <p className="card-description">{clipboardSummary(settings)}</p>
          </div>
          <Button
            variant={settings.paused ? 'primary' : 'secondary'}
            icon={settings.paused ? Play : Pause}
            onClick={() => changeSettings({ paused: !settings.paused }, settings.paused ? 'Sincronização retomada.' : 'Sincronização pausada.')}
          >
            {settings.paused ? 'Retomar' : 'Pausar'}
          </Button>
        </header>
        <SyncFlow settings={settings} />
        <div className="setting-list">
          <SettingSwitch
            icon={ArrowUpRight}
            label="Do PC para o celular"
            checked={settings.upload}
            onChange={upload => changeSettings({ upload })}
          />
          <SettingSwitch
            icon={ArrowDownLeft}
            label="Do celular para o PC"
            checked={settings.download}
            onChange={download => changeSettings({ download })}
          />
        </div>
        <p className="card-footnote">O celular precisa ativar o mesmo sentido. Senhas copiadas de gerenciadores não são enviadas.</p>

        <div className="subsection">
          <h3 className="subsection-title">
            <History aria-hidden />
            Últimas transferências
          </h3>
          {status.transfers.length > 0 ? (
            <ul className="transfer-list">
              {status.transfers.map((entry, index) => (
                <TransferRow key={`${index}-${entry.at}`} entry={entry} />
              ))}
            </ul>
          ) : (
            <p className="muted subsection-empty">Nenhuma transferência ainda.</p>
          )}
          <p className="card-footnote">O texto copiado não é salvo.</p>
        </div>
      </section>

      <section className="card" aria-labelledby="files-title">
        <header className="card-header">
          <h2 id="files-title" className="card-title">
            Arquivos do celular
          </h2>
          <span className={cx('badge', filesTone)}>{filesSummary(settings, status.incomingFile)}</span>
        </header>
        <div className="setting-list">
          <SettingSwitch
            icon={FileImage}
            label="Receber imagens e PDFs do celular"
            checked={settings.allowFiles}
            onChange={allowFiles => changeSettings({ allowFiles })}
          />
        </div>
        {status.incomingFile ? <IncomingRow file={status.incomingFile} /> : null}
        <div className="subsection">
          <div className="subsection-header">
            <h3 className="subsection-title">
              <History aria-hidden />
              Últimos arquivos
            </h3>
            <Button
              variant="ghost"
              size="sm"
              icon={FolderOpen}
              loading={isBusy('openFolder')}
              onClick={() => void run('openFolder', desktopApi.openReceivedFolder)}
            >
              Abrir pasta
            </Button>
          </div>
          {status.receivedFiles.length > 0 ? (
            <ul className="transfer-list">
              {status.receivedFiles.map((file, index) => (
                <ReceivedRow key={`${index}-${file.at}`} file={file} />
              ))}
            </ul>
          ) : (
            <p className="muted subsection-empty">Nenhum arquivo recebido ainda.</p>
          )}
        </div>
        <ul className="notes">
          <li>Somente PNG, JPG e PDF de até 50 MB.</li>
          <li>Os arquivos são salvos em Downloads\PC Remote e marcados como vindos da internet.</li>
        </ul>
      </section>

      <section className="card" aria-labelledby="input-title">
        <header className="card-header">
          <h2 id="input-title" className="card-title">
            Mouse e teclado
          </h2>
          <span className={cx('badge', inputBadge.tone)}>{inputBadge.text}</span>
        </header>
        <div className="setting-list">
          <SettingSwitch
            icon={MousePointer2}
            label="Permitir mouse e teclado pelo celular"
            checked={settings.allowInput}
            onChange={allowInput => changeSettings({ allowInput })}
          />
        </div>
        <div className="kill-switch">
          <div>
            <p className="kill-title">Interromper controle</p>
            <p className="muted">Também disponível no ícone da bandeja.</p>
          </div>
          <Button
            variant="danger"
            icon={Hand}
            disabled={!status.inputActive}
            loading={isBusy('stopInput')}
            onClick={() => void run('stopInput', desktopApi.stopInput, 'Controle interrompido.')}
          >
            Interromper
          </Button>
        </div>
        <ul className="notes">
          <li>Cliques e texto vão para a janela em foco.</li>
          <li>Apps abertos como administrador e a tela de bloqueio podem não responder.</li>
        </ul>
      </section>
    </div>
  );
}
