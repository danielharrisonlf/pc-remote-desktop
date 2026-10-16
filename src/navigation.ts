import { Activity, ArrowLeftRight, LayoutDashboard, Settings, Smartphone, type LucideIcon } from 'lucide-react';

export type Page = 'overview' | 'devices' | 'transfer' | 'diagnostics' | 'settings';

export interface PageMeta {
  /** Short label for the sidebar. */
  nav: string;
  /** Title shown in the header. */
  title: string;
  icon: LucideIcon;
}

export const pages: Record<Page, PageMeta> = {
  overview: { nav: 'Visão geral', title: 'Visão geral', icon: LayoutDashboard },
  devices: { nav: 'Dispositivos', title: 'Dispositivos', icon: Smartphone },
  transfer: { nav: 'Transferência', title: 'Transferência e controle', icon: ArrowLeftRight },
  diagnostics: { nav: 'Diagnóstico', title: 'Diagnóstico', icon: Activity },
  settings: { nav: 'Configurações', title: 'Configurações', icon: Settings },
};

export const primaryPages: Page[] = ['overview', 'devices', 'transfer', 'diagnostics'];
