import { CircleArrowUp, Download, Monitor, Moon, PanelLeft, Power, RefreshCw, RotateCw, Sun } from 'lucide-react';
import { useEffect, useState } from 'react';
import { desktopApi } from '../api';
import { Button } from '../components/Button';
import { Segmented } from '../components/Segmented';
import { SettingSwitch } from '../components/Switch';
import { UpdateNotes, UpdateProgress } from '../components/UpdatePanel';
import type { ThemePreference } from '../hooks/useTheme';
import { useToast } from '../hooks/useToasts';
import { UPDATE_INTERVALS, type UpdateInterval, type Updater } from '../hooks/useUpdater';
import { isBusy, updateStatus } from '../lib/updates';
import { errorMessage } from '../types';
import type { ViewProps } from './viewProps';

interface SettingsViewProps extends ViewProps {
  theme: ThemePreference;
  onThemeChange: (theme: ThemePreference) => void;
  sidebarCollapsed: boolean;
  onSidebarCollapsedChange: (collapsed: boolean) => void;
  updater: Updater;
}

const intervalLabels: Record<UpdateInterval, string> = { 0: 'Manual', 1: '1 h', 6: '6 h', 24: '24 h' };

export function SettingsView({
  settings,
  changeSettings,
  now,
  theme,
  onThemeChange,
  sidebarCollapsed,
  onSidebarCollapsedChange,
  updater,
}: SettingsViewProps) {
  const toast = useToast();
  const { state } = updater;
  const status = updateStatus(state, now);
  const busy = isBusy(state);
  const [wayland, setWayland] = useState(false);
  const [controlReady, setControlReady] = useState(false);
  const [authorizing, setAuthorizing] = useState(false);
  useEffect(() => {
    let mounted = true;
    const refresh = () => {
      void desktopApi.linuxInputStatus().then(([required, ready]) => {
        if (mounted) { setWayland(required); setControlReady(ready); }
      }).catch(() => {});
    };
    refresh();
    const timer = window.setInterval(refresh, 3000);
    return () => { mounted = false; window.clearInterval(timer); };
  }, []);
  const authorizeControl = async () => {
    setAuthorizing(true);
    try {
      await desktopApi.authorizeLinuxInput();
      const [, ready] = await desktopApi.linuxInputStatus();
      setControlReady(ready);
      if (ready) toast.success('Controle autorizado pelo Ubuntu.');
    } catch (reason) { toast.error(errorMessage(reason)); }
    finally { setAuthorizing(false); }
  };

  // The card already shows a found update; a manual check only confirms the other outcomes.
  const checkNow = () => {
    void updater.check().then(outcome => {
      if (outcome.kind === 'failed') toast.error(outcome.message);
      else if (outcome.kind === 'current') toast.success('Você já está na versão mais recente.');
    });
  };

  const attempt = (action: () => Promise<unknown>) => {
    action().catch(reason => toast.error(errorMessage(reason)));
  };

  return (
    <div className="view">
      <section className="card" aria-labelledby="appearance-title">
        <header className="card-header">
          <h2 id="appearance-title" className="card-title">
            Aparência
          </h2>
        </header>
        <div className="field-row">
          <p className="field-title">Tema</p>
          <Segmented<ThemePreference>
            label="Tema"
            value={theme}
            onChange={onThemeChange}
            options={[
              { value: 'system', label: 'Sistema', icon: Monitor },
              { value: 'light', label: 'Claro', icon: Sun },
              { value: 'dark', label: 'Escuro', icon: Moon },
            ]}
          />
        </div>
        <div className="setting-list">
          <SettingSwitch
            icon={PanelLeft}
            label="Barra lateral compacta"
            description="Mostra só os ícones."
            checked={sidebarCollapsed}
            onChange={onSidebarCollapsedChange}
          />
        </div>
      </section>

      <section className="card" aria-labelledby="system-title">
        <header className="card-header">
          <h2 id="system-title" className="card-title">
            Sistema
          </h2>
        </header>
        <div className="setting-list">
          <SettingSwitch
            icon={Power}
            label="Iniciar com o sistema"
            checked={settings.autostart}
            onChange={autostart => changeSettings({ autostart })}
          />
        </div>
        <p className="card-footnote">Fechar a janela mantém o PC Remote na bandeja. Para sair, use Encerrar no ícone da bandeja.</p>
      </section>

      {wayland && (
        <section className="card" aria-labelledby="ubuntu-control-title">
          <header className="card-header">
            <h2 id="ubuntu-control-title" className="card-title">Controle no Ubuntu</h2>
          </header>
          <p className="card-description">
            Autorize mouse e teclado na janela do sistema para controlar aplicativos pelo celular.
            O Ubuntu pode pedir essa permissão novamente ao reabrir o PC Remote.
          </p>
          <p role="status">{controlReady ? 'Controle autorizado.' : 'Aguardando autorização no computador.'}</p>
          <div className="button-row">
            <Button variant="primary" loading={authorizing} disabled={controlReady} onClick={() => void authorizeControl()}>
              Autorizar controle
            </Button>
          </div>
        </section>
      )}

      <section className="card" aria-labelledby="updates-title">
        <header className="card-header">
          <div>
            <h2 id="updates-title" className="card-title">
              Atualizações
            </h2>
            <p className="card-description">
              Versão <span className="tabular">{updater.currentVersion}</span>
            </p>
          </div>
          <CircleArrowUp className="card-glyph" aria-hidden />
        </header>
        <p className={`update-status ${status.tone}`} role="status">
          {status.text}
        </p>
        <UpdateProgress updater={updater} />
        <div className="button-row">
          {state.phase === 'ready' || state.phase === 'installing' ? (
            <Button variant="primary" size="lg" icon={RotateCw} loading={state.phase === 'installing'} onClick={() => attempt(updater.install)}>
              Reiniciar para atualizar
            </Button>
          ) : state.phase === 'available' ? (
            <Button variant="primary" icon={Download} onClick={() => attempt(updater.downloadAndInstall)}>
              Baixar e instalar
            </Button>
          ) : state.phase === 'error' ? (
            <Button variant="primary" icon={RefreshCw} onClick={() => attempt(updater.retry)}>
              Tentar de novo
            </Button>
          ) : null}
          {state.phase !== 'error' ? (
            <Button icon={RefreshCw} loading={state.phase === 'checking'} disabled={busy} onClick={checkNow}>
              Verificar agora
            </Button>
          ) : null}
        </div>
        <UpdateNotes updater={updater} />
        <div className="field-row">
          <p className="field-title">Verificar automaticamente</p>
          <Segmented<UpdateInterval>
            label="Intervalo de verificação"
            value={updater.interval}
            onChange={updater.setInterval}
            options={UPDATE_INTERVALS.map(value => ({ value, label: intervalLabels[value] }))}
          />
        </div>
        <div className="setting-list">
          <SettingSwitch
            icon={Download}
            label="Baixar atualizações automaticamente"
            description="Baixa em segundo plano e confere a assinatura. A instalação só começa quando você reiniciar."
            checked={updater.autoDownload}
            onChange={updater.setAutoDownload}
          />
        </div>
      </section>
    </div>
  );
}
