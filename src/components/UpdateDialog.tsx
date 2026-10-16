import { Download, ExternalLink } from 'lucide-react';
import { desktopApi } from '../api';
import type { Updater } from '../hooks/useUpdater';
import { useToast } from '../hooks/useToasts';
import { formatBytes, formatLongDate } from '../lib/format';
import { errorMessage } from '../types';
import { Button } from './Button';
import { Dialog } from './Dialog';
import { Markdown } from './Markdown';

const openableLink = (href: string) => /^https:\/\/github\.com\/danielharrisonlf\/pc-remote-(desktop|android)\/releases(\/|$)/.test(href);

export function UpdateDialog({ open, onClose, updater }: { open: boolean; onClose: () => void; updater: Updater }) {
  const toast = useToast();
  const { release, installer, currentVersion, available } = updater;
  if (!release) return null;

  const published = formatLongDate(release.publishedAt);
  const description = available
    ? `${published ? `Publicada em ${published}. ` : ''}Você usa a versão ${currentVersion}.`
    : 'Você já tem esta versão.';

  const attempt = (action: () => Promise<void>, success?: string) => {
    action()
      .then(() => success && toast.info(success))
      .catch(reason => toast.error(errorMessage(reason)));
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      size="lg"
      title={available ? `PC Remote ${release.version} disponível` : `Notas da versão ${release.version}`}
      description={description}
      footer={
        <>
          {available ? (
            <Button
              variant="ghost"
              className="footer-start"
              onClick={() => {
                updater.skip(release.version);
                onClose();
              }}
            >
              Ignorar esta versão
            </Button>
          ) : null}
          <Button icon={ExternalLink} onClick={() => attempt(updater.openReleasePage)}>
            Ver no GitHub
          </Button>
          {available ? (
            <Button
              variant="primary"
              icon={Download}
              onClick={() => attempt(updater.download, 'Download iniciado no navegador.')}
            >
              {installer ? `Baixar instalador${installer.size ? ` (${formatBytes(installer.size)})` : ''}` : 'Abrir página de download'}
            </Button>
          ) : null}
        </>
      }
    >
      <div className="release-notes">
        <Markdown source={release.notes} canOpen={openableLink} onOpenLink={href => attempt(() => desktopApi.openUrl(href))} />
      </div>
      {available ? <p className="release-hint">O instalador não tem assinatura digital, então o Windows pode exibir um aviso.</p> : null}
    </Dialog>
  );
}
