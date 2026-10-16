import { CircleArrowUp, Moon, RefreshCw, Settings, Sun } from 'lucide-react';
import type { ServiceState } from '../hooks/useStatus';
import type { ResolvedTheme } from '../hooks/useTheme';
import { cx } from '../lib/cx';
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
  theme: ResolvedTheme;
  onToggleTheme: () => void;
  onOpenSettings: () => void;
  updateVersion: string | null;
  onOpenUpdate: () => void;
}

export function Header({ title, serviceState, serviceDetail, onRetry, theme, onToggleTheme, onOpenSettings, updateVersion, onOpenUpdate }: HeaderProps) {
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
        {updateVersion ? (
          <button type="button" className="update-pill" onClick={onOpenUpdate} aria-label={`Versão ${updateVersion} disponível`} title="Ver novidades">

            <CircleArrowUp aria-hidden />
            <span>
              Versão <span className="tabular">{updateVersion}</span> disponível
            </span>
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
        <IconButton label={theme === 'dark' ? 'Usar tema claro' : 'Usar tema escuro'} icon={theme === 'dark' ? Sun : Moon} onClick={onToggleTheme} />
        <IconButton label="Configurações" icon={Settings} onClick={onOpenSettings} />
      </div>
    </header>
  );
}
