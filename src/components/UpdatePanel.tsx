import { ExternalLink, ShieldCheck } from 'lucide-react';
import { desktopApi } from '../api';
import type { Updater } from '../hooks/useUpdater';
import { useToast } from '../hooks/useToasts';
import { formatBytes, formatLongDate } from '../lib/format';
import { downloadRatio, isReleaseLink } from '../lib/updates';
import { errorMessage } from '../types';
import { Markdown } from './Markdown';

/**
 * What an update looks like right now: download progress, a plain error with the release
 * page as fallback and the restart hint. Shared by the settings card and the update dialog;
 * the actions live with each host.
 */
export function UpdateProgress({ updater }: { updater: Updater }) {
  const toast = useToast();
  const { state } = updater;
  const ratio = downloadRatio(state);
  const busy = state.phase === 'downloading' || state.phase === 'verifying';

  return (
    <div className="update-panel">
      {busy ? (
        <div className="update-progress">
          <div
            className="progress-track"
            role="progressbar"
            aria-label={`Baixando a versão ${state.update?.version ?? ''}`}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={ratio === null ? undefined : Math.round(ratio * 100)}
          >
            <span className={ratio === null ? 'is-indeterminate' : undefined} style={{ transform: `scaleX(${ratio ?? 0.3})` }} />
          </div>
          <p className="update-progress-meta tabular">
            {state.phase === 'verifying'
              ? 'Conferindo a assinatura…'
              : state.total
                ? `${formatBytes(state.downloaded) || '0 B'} de ${formatBytes(state.total)}`
                : formatBytes(state.downloaded) || 'Iniciando…'}
          </p>
        </div>
      ) : null}

      {state.phase === 'error' && state.error && state.error.stage !== 'check' ? (
        <div className="update-error" role="alert">
          <p>{state.error.message}</p>
          <button type="button" className="card-link" onClick={() => updater.openReleasePage().catch(reason => toast.error(errorMessage(reason)))}>
            Baixar o instalador pela página da versão
            <ExternalLink aria-hidden />
          </button>
        </div>
      ) : null}

      {state.phase === 'ready' ? (
        <p className="update-hint">
          <ShieldCheck aria-hidden />
          <span>
            Ao reiniciar, o PC Remote fecha, instala sem perguntas e abre de novo em alguns segundos. A conexão com o celular cai durante a
            instalação.
          </span>
        </p>
      ) : null}
    </div>
  );
}

/** Release notes from latest.json; only this project's release links can be opened. */
export function UpdateNotes({ updater }: { updater: Updater }) {
  const toast = useToast();
  const update = updater.state.update;
  if (!update) return null;
  return (
    <section className="update-notes" aria-label={`Novidades da versão ${update.version}`}>
      <h3 className="update-notes-title">
        Novidades da versão <span className="tabular">{update.version}</span>
        {update.publishedAt ? <span className="update-notes-date"> · {formatLongDate(new Date(update.publishedAt).toISOString())}</span> : null}
      </h3>
      <div className="release-notes">
        <Markdown
          source={update.notes}
          canOpen={isReleaseLink}
          onOpenLink={href => desktopApi.openUrl(href).catch(reason => toast.error(errorMessage(reason)))}
        />
      </div>
    </section>
  );
}
