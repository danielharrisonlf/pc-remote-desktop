import { CircleArrowUp, Monitor, Moon, PanelLeft, Power, RefreshCw, Sun } from 'lucide-react';
import { Button } from '../components/Button';
import { Segmented } from '../components/Segmented';
import { SettingSwitch } from '../components/Switch';
import type { ThemePreference } from '../hooks/useTheme';
import { useToast } from '../hooks/useToasts';
import { UPDATE_INTERVALS, type UpdateInterval, type Updater } from '../hooks/useUpdater';
import { formatRelative } from '../lib/format';
import { isNewerVersion } from '../lib/semver';
import type { ViewProps } from './viewProps';

interface SettingsViewProps extends ViewProps {
  theme: ThemePreference;
  onThemeChange: (theme: ThemePreference) => void;
  sidebarCollapsed: boolean;
  onSidebarCollapsedChange: (collapsed: boolean) => void;
  updater: Updater;
  openUpdate: () => void;
}

const intervalLabels: Record<UpdateInterval, string> = { 0: 'Manual', 1: '1 h', 6: '6 h', 24: '24 h' };

function updateStatus(updater: Updater, now: number): { text: string; tone: 'muted' | 'text-success' | 'text-accent' | 'text-warning' } {
  if (updater.checking) return { text: 'Verificando…', tone: 'muted' };
  const checked = updater.checkedAt ? ` Última verificação ${formatRelative(updater.checkedAt, now)}.` : '';
  const lookup = updater.lookup;
  if (!lookup) return { text: 'Ainda não verificado.', tone: 'muted' };
  if (updater.available && updater.release) return { text: `A versão ${updater.release.version} está disponível.${checked}`, tone: 'text-accent' };
  if (lookup.kind === 'unavailable') return { text: `${lookup.message}${checked}`, tone: 'text-warning' };
  if (lookup.kind === 'none') return { text: `Nenhuma versão publicada ainda.${checked}`, tone: 'muted' };
  return { text: `Você está na versão mais recente.${checked}`, tone: 'text-success' };
}

export function SettingsView({
  settings,
  changeSettings,
  now,
  theme,
  onThemeChange,
  sidebarCollapsed,
  onSidebarCollapsedChange,
  updater,
  openUpdate,
}: SettingsViewProps) {
  const toast = useToast();
  const state = updateStatus(updater, now);

  // A manual check answers right away: open the notes when there is something
  // new, otherwise confirm with a toast.
  const checkNow = () => {
    void updater.check().then(lookup => {
      if (!lookup) return;
      if (lookup.kind === 'unavailable') toast.error(lookup.message);
      else if (lookup.kind === 'found' && isNewerVersion(lookup.release.version, updater.currentVersion)) openUpdate();
      else toast.success('Você já está na versão mais recente.');
    });
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
          <div>
            <p className="field-title">Tema</p>
            <p className="muted">“Sistema” acompanha o modo claro ou escuro do Windows.</p>
          </div>
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
            description="Mostra só os ícones. Em janelas estreitas ela fica compacta automaticamente."
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
            label="Iniciar com o Windows"
            description="Deixa o serviço disponível na bandeja assim que você entra na sua conta."
            checked={settings.autostart}
            onChange={autostart => changeSettings({ autostart }, autostart ? 'O PC Remote vai iniciar com o Windows.' : 'Inicialização automática desligada.')}
          />
        </div>
        <p className="card-footnote">Fechar esta janela mantém o serviço na bandeja. Para encerrar de vez, use Encerrar no menu do ícone da bandeja.</p>
      </section>

      <section className="card" aria-labelledby="updates-title">
        <header className="card-header">
          <div>
            <h2 id="updates-title" className="card-title">
              Atualizações
            </h2>
            <p className="card-description">
              Versão instalada <span className="tabular">{updater.currentVersion}</span>. As novidades vêm das releases do GitHub; nada é
              instalado sem você confirmar.
            </p>
          </div>
          <CircleArrowUp className="card-glyph" aria-hidden />
        </header>
        <p className={`update-status ${state.tone}`} role="status">
          {state.text}
        </p>
        <div className="field-row">
          <div>
            <p className="field-title">Verificar automaticamente</p>
            <p className="muted">Ao abrir o aplicativo e depois no intervalo escolhido.</p>
          </div>
          <Segmented<UpdateInterval>
            label="Intervalo de verificação"
            value={updater.interval}
            onChange={updater.setInterval}
            options={UPDATE_INTERVALS.map(value => ({ value, label: intervalLabels[value] }))}
          />
        </div>
        <div className="button-row">
          <Button icon={RefreshCw} loading={updater.checking} onClick={checkNow}>
            Verificar agora
          </Button>
          {updater.release ? (
            <Button variant={updater.available ? 'primary' : 'ghost'} onClick={openUpdate}>
              {updater.available ? 'Ver novidades e baixar' : 'Notas da versão atual'}
            </Button>
          ) : null}
        </div>
      </section>
    </div>
  );
}
