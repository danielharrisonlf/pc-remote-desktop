import { Download, ExternalLink, RefreshCw, RotateCw } from 'lucide-react';
import type { Updater } from '../hooks/useUpdater';
import { useToast } from '../hooks/useToasts';
import { formatLongDate } from '../lib/format';
import { errorMessage } from '../types';
import { Button } from './Button';
import { Dialog } from './Dialog';
import { UpdateNotes, UpdateProgress } from './UpdatePanel';

/** Opened from the header notice: the found update, its progress and the next step. */
export function UpdateDialog({ open, onClose, updater }: { open: boolean; onClose: () => void; updater: Updater }) {
  const toast = useToast();
  const { state, currentVersion } = updater;
  const update = state.update;
  if (!update) return null;

  const { phase } = state;
  const busy = phase === 'downloading' || phase === 'verifying' || phase === 'installing';
  const published = update.publishedAt ? formatLongDate(new Date(update.publishedAt).toISOString()) : null;
  const title =
    phase === 'error'
      ? 'Não foi possível atualizar'
      : phase === 'ready'
        ? 'Atualização pronta'
        : phase === 'installing'
          ? `Instalando PC Remote ${update.version}`
          : busy
            ? `Baixando PC Remote ${update.version}`
            : `PC Remote ${update.version} disponível`;

  const attempt = (action: () => Promise<unknown>) => {
    action().catch(reason => toast.error(errorMessage(reason)));
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      size="lg"
      title={title}
      description={`Versão ${update.version}${published ? `, publicada em ${published}` : ''}. Você usa a ${currentVersion}.`}
      footer={
        <>
          <Button variant="ghost" icon={ExternalLink} className="footer-start" onClick={() => attempt(updater.openReleasePage)}>
            Ver no GitHub
          </Button>
          <Button onClick={onClose}>{phase === 'ready' ? 'Mais tarde' : 'Fechar'}</Button>
          {phase === 'ready' ? (
            <Button variant="primary" icon={RotateCw} onClick={() => attempt(updater.install)}>
              Reiniciar para atualizar
            </Button>
          ) : busy ? (
            <Button variant="primary" loading>
              {phase === 'installing' ? 'Instalando…' : phase === 'verifying' ? 'Verificando…' : 'Baixando…'}
            </Button>
          ) : phase === 'error' ? (
            <Button variant="primary" icon={RefreshCw} onClick={() => attempt(updater.retry)}>
              Tentar de novo
            </Button>
          ) : (
            <Button variant="primary" icon={Download} onClick={() => attempt(updater.downloadAndInstall)}>
              Baixar e instalar
            </Button>
          )}
        </>
      }
    >
      <div className="update-dialog-body">
        <UpdateProgress updater={updater} />
        <UpdateNotes updater={updater} />
      </div>
    </Dialog>
  );
}
