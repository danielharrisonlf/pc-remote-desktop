import { CircleArrowUp, LoaderCircle, RefreshCw, RotateCw, Settings } from 'lucide-react';
import type { ServiceState } from '../hooks/useStatus';
import { cx } from '../lib/cx';
import type { UpdateNotice } from '../lib/updates';
import { IconButton } from './Button';

const serviceCopy: Record<ServiceState, string> = {
  starting: 'Conectando',
  active: 'Ativo',
  connected: 'Celular conectado',
  unavailable: 'Serviço indisponível',
};

interface HeaderProps {
  title: string;
  serviceState: ServiceState;
  serviceDetail: string | null;
  onRetry: () => void;
  onOpenSettings: () => void;
  /** Null hides the update pill. */
  update: UpdateNotice | null;
  onOpenUpdate: () => void;
}

const noticeIcon = { available: CircleArrowUp, busy: LoaderCircle, ready: RotateCw };

/** The theme lives in Configurações only; a second toggle here repeated it. */
export function Header({ title, serviceState, serviceDetail, onRetry, onOpenSettings, update, onOpenUpdate }: HeaderProps) {
  const NoticeIcon = update ? noticeIcon[update.tone] : null;
  const unavailable = serviceState === 'unavailable';
  const pill = (
    <>
      <span className="status-dot" aria-hidden />
      <span>{serviceCopy[serviceState]}</span>
      {unavailable ? <RefreshCw className="status-retry" aria-hidden /> : null}
    </>
  );
  return (
    <header className="topbar">
      <h1 className="page-title">{title}</h1>
      <div className="topbar-actions">
        {update && NoticeIcon ? (
          <button
            type="button"
            className={cx('update-pill', update.tone === 'ready' && 'is-ready')}
            onClick={onOpenUpdate}
            aria-label={update.title}
            title={update.title}
          >
            <NoticeIcon className={update.tone === 'busy' ? 'spin' : undefined} aria-hidden />
            <span className="tabular">{update.label}</span>
          </button>
        ) : null}
        {unavailable ? (
          <button
            type="button"
            className={cx('status-pill', `status-${serviceState}`)}
            onClick={onRetry}
            title={serviceDetail ? `${serviceDetail} Clique para tentar novamente.` : 'Clique para tentar novamente.'}
          >
            {pill}
          </button>
        ) : (
          <span className={cx('status-pill', `status-${serviceState}`)} role="status">
            {pill}
          </span>
        )}
        <span className="topbar-divider" aria-hidden />
        <IconButton label="Configurações" icon={Settings} onClick={onOpenSettings} />
      </div>
    </header>
  );
}
