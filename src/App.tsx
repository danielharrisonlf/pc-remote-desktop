import { useCallback, useEffect, useRef, useState } from 'react';
import { LoaderCircle, RefreshCw, ServerOff } from 'lucide-react';
import { Button } from './components/Button';
import { EmptyState } from './components/EmptyState';
import { Header } from './components/Header';
import { PairingDrawer } from './components/PairingDrawer';
import { Sidebar } from './components/Sidebar';
import { ToastViewport } from './components/Toasts';
import { UpdateDialog } from './components/UpdateDialog';
import { useActions, useSettings } from './hooks/useActions';
import { useMediaQuery } from './hooks/useMediaQuery';
import { useNow } from './hooks/useNow';
import { useStatus } from './hooks/useStatus';
import { useTheme } from './hooks/useTheme';
import { ToastProvider, useToast } from './hooks/useToasts';
import { useUpdater } from './hooks/useUpdater';
import { isBoolean, readStored, writeStored } from './lib/storage';
import { updateNotice } from './lib/updates';
import { pages, type Page } from './navigation';
import { DevicesView } from './views/DevicesView';
import { DiagnosticsView } from './views/DiagnosticsView';
import { OverviewView } from './views/OverviewView';
import { SettingsView } from './views/SettingsView';
import { TransferView } from './views/TransferView';
import type { ViewProps } from './views/viewProps';

export default function App() {
  return (
    <ToastProvider>
      <Shell />
      <ToastViewport />
    </ToastProvider>
  );
}

function Shell() {
  const [page, setPage] = useState<Page>('overview');
  const [pairingOpen, setPairingOpen] = useState(false);
  const [updateOpen, setUpdateOpen] = useState(false);
  const [userCollapsed, setUserCollapsed] = useState(() => readStored('sidebar.collapsed', false, isBoolean));
  const narrow = useMediaQuery('(max-width: 879px)');
  const toast = useToast();
  const theme = useTheme();
  const updater = useUpdater();
  // Poll faster while the pairing drawer waits for a phone.
  const { status, serviceState, lastError, refresh } = useStatus(pairingOpen);
  const { run, isBusy } = useActions(refresh);
  const { settings, change: changeSettings } = useSettings(status, refresh);
  const now = useNow(1000);
  const mainRef = useRef<HTMLElement>(null);

  // Announce the service dropping once per outage, not on every failed poll.
  const wasUnavailable = useRef(false);
  useEffect(() => {
    const unavailable = serviceState === 'unavailable';
    if (unavailable && !wasUnavailable.current && status) toast.error('Serviço sem resposta. Tentando reconectar…');
    if (!unavailable && wasUnavailable.current) toast.success('Serviço reconectado.');
    wasUnavailable.current = unavailable;
  }, [serviceState, status, toast]);

  // A new pairing request deserves attention even from another page.
  const knownRequests = useRef(new Set<string>());
  useEffect(() => {
    if (!status) return;
    for (const request of status.pending) {
      if (knownRequests.current.has(request.id)) continue;
      knownRequests.current.add(request.id);
      if (!pairingOpen && page !== 'devices' && page !== 'overview') {
        toast.info(`${request.deviceName} pediu acesso a este PC.`, { label: 'Revisar', run: () => setPage('devices') });
      }
    }
  }, [status, pairingOpen, page, toast]);

  const navigate = useCallback((next: Page) => {
    setPage(next);
    mainRef.current?.scrollTo({ top: 0 });
  }, []);

  const setCollapsed = useCallback((collapsed: boolean) => {
    writeStored('sidebar.collapsed', collapsed);
    setUserCollapsed(collapsed);
  }, []);

  const viewProps: ViewProps | null =
    status && settings
      ? { status, settings, changeSettings, run, isBusy, serviceState, now, navigate, openPairing: () => setPairingOpen(true) }
      : null;

  return (
    <div className="app-shell">
      <Sidebar
        page={page}
        onNavigate={navigate}
        collapsed={narrow || userCollapsed}
        canToggle={!narrow}
        onToggle={() => setCollapsed(!userCollapsed)}
        pendingCount={status?.pending.length ?? 0}
        inputActive={status?.inputActive ?? false}
        updateAvailable={updater.state.update !== null}
        version={updater.currentVersion}
      />
      <div className="main-column">
        <Header
          title={pages[page].title}
          serviceState={serviceState}
          serviceDetail={lastError}
          onRetry={() => void refresh()}
          theme={theme.resolved}
          onToggleTheme={theme.toggle}
          onOpenSettings={() => navigate('settings')}
          update={updateNotice(updater.state)}
          onOpenUpdate={() => setUpdateOpen(true)}
        />
        <main ref={mainRef} className="content" id="main">
          {viewProps ? (
            page === 'overview' ? (
              <OverviewView {...viewProps} />
            ) : page === 'devices' ? (
              <DevicesView {...viewProps} />
            ) : page === 'transfer' ? (
              <TransferView {...viewProps} />
            ) : page === 'diagnostics' ? (
              <DiagnosticsView {...viewProps} appVersion={updater.currentVersion} refresh={refresh} />
            ) : (
              <SettingsView
                {...viewProps}
                theme={theme.preference}
                onThemeChange={theme.setPreference}
                sidebarCollapsed={userCollapsed}
                onSidebarCollapsedChange={setCollapsed}
                updater={updater}
              />
            )
          ) : serviceState === 'unavailable' ? (
            <div className="view view-center">
              <EmptyState
                icon={ServerOff}
                title="O serviço não respondeu"
                action={
                  <Button variant="primary" icon={RefreshCw} onClick={() => void refresh()}>
                    Tentar novamente
                  </Button>
                }
              >
                Se continuar, encerre pelo ícone da bandeja e abra o PC Remote de novo.
              </EmptyState>
            </div>
          ) : (
            <div className="view view-center" role="status">
              <p className="loading-line">
                <LoaderCircle className="spin" aria-hidden />
                Carregando…
              </p>
            </div>
          )}
        </main>
      </div>
      <PairingDrawer open={pairingOpen} onClose={() => setPairingOpen(false)} status={status} run={run} isBusy={isBusy} />
      <UpdateDialog open={updateOpen} onClose={() => setUpdateOpen(false)} updater={updater} />
    </div>
  );
}
