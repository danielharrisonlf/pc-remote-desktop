import { PanelLeftClose, PanelLeftOpen } from 'lucide-react';
import brandIcon from '../assets/brand.svg';
import { cx } from '../lib/cx';
import { pages, primaryPages, type Page } from '../navigation';

interface SidebarProps {
  page: Page;
  onNavigate: (page: Page) => void;
  collapsed: boolean;
  /** Hidden when the window is too narrow for the expanded sidebar. */
  canToggle: boolean;
  onToggle: () => void;
  pendingCount: number;
  inputActive: boolean;
  updateAvailable: boolean;
  version: string;
}

export function Sidebar({ page, onNavigate, collapsed, canToggle, onToggle, pendingCount, inputActive, updateAvailable, version }: SidebarProps) {
  const indicator = (target: Page) => {
    if (target === 'devices' && pendingCount > 0) {
      const label = `${pendingCount} ${pendingCount === 1 ? 'pedido pendente' : 'pedidos pendentes'}`;
      return (
        <span className="nav-badge" aria-label={label} title={label}>
          {pendingCount}
        </span>
      );
    }
    if (target === 'transfer' && inputActive) return <span className="nav-dot nav-dot-success" aria-label="Controle remoto ativo" title="Controle remoto ativo" />;
    if (target === 'settings' && updateAvailable) return <span className="nav-dot nav-dot-accent" aria-label="Atualização disponível" title="Atualização disponível" />;
    return null;
  };

  const item = (target: Page) => {
    const { nav, icon: Icon } = pages[target];
    const active = page === target;
    return (
      <li key={target}>
        <button
          type="button"
          className={cx('nav-item', active && 'is-active')}
          aria-current={active ? 'page' : undefined}
          title={collapsed ? nav : undefined}
          onClick={() => onNavigate(target)}
        >
          <Icon className="nav-icon" aria-hidden />
          <span className="nav-label">{nav}</span>
          {indicator(target)}
        </button>
      </li>
    );
  };

  return (
    <aside className={cx('sidebar', collapsed && 'is-collapsed')}>
      <div className="brand">
        <span className="brand-mark" aria-hidden>
          <img src={brandIcon} alt="" />
        </span>
        <span className="brand-name">PC Remote</span>
      </div>
      <nav aria-label="Navegação principal">
        <ul className="nav-list">{primaryPages.map(item)}</ul>
      </nav>
      <div className="sidebar-footer">
        <ul className="nav-list">{item('settings')}</ul>
        {canToggle ? (
          <button
            type="button"
            className="nav-item nav-item-quiet"
            onClick={onToggle}
            aria-label={collapsed ? 'Expandir barra lateral' : 'Recolher barra lateral'}
            title={collapsed ? 'Expandir barra lateral' : undefined}
          >
            {collapsed ? <PanelLeftOpen className="nav-icon" aria-hidden /> : <PanelLeftClose className="nav-icon" aria-hidden />}
            <span className="nav-label">Recolher</span>
          </button>
        ) : null}
        <p className="sidebar-version">
          <span className="nav-label">Versão </span>
          <span className="tabular">{version}</span>
        </p>
      </div>
    </aside>
  );
}
