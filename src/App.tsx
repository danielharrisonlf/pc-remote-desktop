import { useCallback, useEffect, useRef, useState } from 'react';
import { toDataURL } from 'qrcode';
import {
  ArrowDownToLine, ArrowUpFromLine, Check, ChevronRight, CircleHelp, Clipboard,
  Computer, Copy, LockKeyhole, MonitorSmartphone, Pause, Play, Plus, Power,
  RefreshCw, Settings2, ShieldCheck, ShieldX, Smartphone, Wifi, WifiOff,
} from 'lucide-react';
import { desktopApi } from './api';
import { errorMessage, formatExpiry, preferredAddress } from './types';
import type { DesktopStatus, Settings } from './types';

type Page = 'overview' | 'devices' | 'settings' | 'diagnostics';

const pageLabels: Record<Page, string> = {
  overview: 'Visão geral', devices: 'Dispositivos', settings: 'Preferências', diagnostics: 'Diagnóstico',
};

function Toggle({ checked, disabled, label, detail, onChange }: {
  checked: boolean; disabled?: boolean; label: string; detail: string; onChange: (value: boolean) => void;
}) {
  return <label className={`setting-row ${disabled ? 'is-disabled' : ''}`}>
    <span><strong>{label}</strong><small>{detail}</small></span>
    <input type="checkbox" checked={checked} disabled={disabled} onChange={event => onChange(event.target.checked)} />
    <span className="switch" aria-hidden="true" />
  </label>;
}

function App() {
  const [page, setPage] = useState<Page>('overview');
  const [status, setStatus] = useState<DesktopStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [qrImage, setQrImage] = useState<string | null>(null);
  const latestRefresh = useRef(0);
  const [theme, setTheme] = useState<'light' | 'dark'>(() =>
    window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
  );

  const refresh = useCallback(async (silent = false) => {
    const requestNumber = ++latestRefresh.current;
    try {
      const next = await desktopApi.getStatus();
      if (requestNumber === latestRefresh.current) {
        setStatus(next);
        setError(null);
      }
    } catch (reason) {
      if (requestNumber === latestRefresh.current) setError(errorMessage(reason));
    } finally {
      if (!silent) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => { void refresh(true); }, 2000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  useEffect(() => {
    if (!status?.pairing?.qrPayload) { setQrImage(null); return; }
    let active = true;
    void toDataURL(status.pairing.qrPayload, { width: 240, margin: 1, errorCorrectionLevel: 'M' })
      .then(data => { if (active) setQrImage(data); })
      .catch(() => { if (active) setQrImage(null); });
    return () => { active = false; };
  }, [status?.pairing?.qrPayload]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  async function runAction(key: string, action: () => Promise<unknown>, success: string) {
    setBusy(key);
    setMessage(null);
    setError(null);
    try {
      await action();
      await refresh(true);
      setMessage(success);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(null);
    }
  }

  async function changeSettings(patch: Partial<Settings>) {
    if (!status) return;
    const settings = { ...status.settings, ...patch };
    await runAction('settings', () => desktopApi.updateSettings(settings), 'Preferências atualizadas.');
  }

  async function copyText(value: string, label: string) {
    try {
      await navigator.clipboard.writeText(value);
      setMessage(`${label} copiado.`);
    } catch {
      setError('Selecione e copie o texto manualmente.');
    }
  }

  const address = preferredAddress(status?.addresses ?? []);
  const connected = status?.devices.filter(device => device.connected).length ?? 0;
  const pairing = status?.pairing;
  const serviceOffline = Boolean(error) || status?.serviceRunning === false;

  return <div className="app-shell">
    <aside className="sidebar">
      <div className="brand"><span className="brand-mark"><MonitorSmartphone size={22} strokeWidth={1.8} /></span><span>PC Remote<small>CONTROLE LOCAL</small></span></div>
      <nav aria-label="Navegação principal">
        <button className={page === 'overview' ? 'active' : ''} onClick={() => setPage('overview')}><Computer size={18} />Visão geral</button>
        <button className={page === 'devices' ? 'active' : ''} onClick={() => setPage('devices')}><Smartphone size={18} />Dispositivos{status?.pending.length ? <em>{status.pending.length}</em> : null}</button>
        <button className={page === 'settings' ? 'active' : ''} onClick={() => setPage('settings')}><Settings2 size={18} />Preferências</button>
        <button className={page === 'diagnostics' ? 'active' : ''} onClick={() => setPage('diagnostics')}><CircleHelp size={18} />Diagnóstico</button>
      </nav>
      <div className="sidebar-bottom">
        <div className="local-badge"><LockKeyhole size={15} /> Rede local · criptografado</div>
        <button className="theme-button" onClick={() => setTheme(theme === 'light' ? 'dark' : 'light')}>Tema {theme === 'light' ? 'escuro' : 'claro'}</button>
        <span className="version">DESKTOP 0.2.0 · PROTOCOLO 1</span>
      </div>
    </aside>

    <main className="main-content">
      <header className="topbar"><div className="breadcrumbs">PC REMOTE <ChevronRight size={13} /> {pageLabels[page].toUpperCase()}</div><div className={`connection-pill ${serviceOffline ? 'offline' : ''}`}><span />{serviceOffline ? 'Serviço indisponível' : status ? 'Serviço ativo' : 'Conectando...'}</div></header>
      <div className="content">
        {error && <div className="notice error" role="alert"><WifiOff size={18} /><span>{error}</span><button onClick={() => { void refresh(); }} aria-label="Tentar novamente"><RefreshCw size={17} /></button></div>}
        {message && <div className="notice success" role="status"><Check size={18} /><span>{message}</span><button onClick={() => setMessage(null)} aria-label="Fechar mensagem">×</button></div>}
        {loading && !status ? <div className="loading"><RefreshCw className="spin" size={24} /> Lendo estado do serviço…</div> : null}
        {!status ? null : page === 'overview' ? <>
          <div className="page-heading"><div><p className="eyebrow">SEU COMPUTADOR</p><h1>{status.pcName}</h1><p>Pronto para receber conexões na sua rede local.</p></div><span className="hero-icon"><Computer size={32} strokeWidth={1.5} /></span></div>
          <div className="metric-grid">
            <div className="metric"><span className="metric-icon teal"><Wifi size={21} /></span><small>CONEXÃO</small><strong>{connected > 0 ? `${connected} conectado${connected > 1 ? 's' : ''}` : 'Aguardando celular'}</strong><span>{address ? `${address}:${status.port}` : 'Sem endereço de rede local'}</span></div>
            <div className="metric"><span className="metric-icon blue"><Smartphone size={21} /></span><small>DISPOSITIVOS</small><strong>{status.devices.length} pareado{status.devices.length === 1 ? '' : 's'}</strong><span>{status.pending.length ? `${status.pending.length} aguardando aprovação` : 'Acesso sob seu controle'}</span></div>
            <div className="metric"><span className="metric-icon amber"><Clipboard size={21} /></span><small>ÁREA DE TRANSFERÊNCIA</small><strong>{status.settings.paused ? 'Pausada' : status.settings.upload || status.settings.download ? 'Ativa' : 'Desativada'}</strong><span>{status.settings.upload ? 'PC → celular' : 'PC → celular desativado'} · {status.settings.download ? 'celular → PC' : 'celular → PC desativado'}</span></div>
          </div>
          <div className="section-grid"><section className="panel pairing-panel"><div className="panel-title"><div><p className="eyebrow">COMECE AQUI</p><h2>Parear um celular</h2></div><span className="soft-icon"><Plus size={20} /></span></div>
            <p>Abra o PC Remote no Android e escaneie o QR Code. Confirme o pedido neste computador antes de liberar o acesso.</p>
            {pairing ? <><div className="pairing-active"><div className="qr-box">{qrImage ? <img src={qrImage} alt="QR Code de pareamento com endereço e identidade deste PC" /> : <RefreshCw className="spin" size={22} />}</div><div className="pairing-details"><span className="eyebrow">CÓDIGO TEMPORÁRIO</span><strong className="code" aria-label={`Código ${pairing.code}`}>{pairing.code}</strong><span>Válido até {formatExpiry(pairing.expiresAt)}</span><button className="text-button" onClick={() => { void copyText(pairing.qrPayload, 'Dados de pareamento'); }}><Copy size={15} /> Copiar dados para parear</button></div></div><div className="manual-pairing"><strong>Prefere digitar no celular?</strong><span>Endereço: {address ? `${address}:${status.port}` : 'indisponível'}</span><span>Confira a identidade do PC no Android:</span><code>{status.fingerprint}</code></div></> : <button className="primary-button" disabled={busy !== null} onClick={() => { void runAction('pair', desktopApi.startPairing, 'Pareamento aberto por dois minutos.'); }}><Plus size={18} />{busy === 'pair' ? 'Preparando…' : 'Adicionar celular'}</button>}
          </section><section className="panel quick-panel"><div className="panel-title"><div><p className="eyebrow">ACESSO RÁPIDO</p><h2>Sincronização</h2></div><span className="soft-icon"><Clipboard size={20} /></span></div><p>Envie texto entre os dispositivos apenas nas direções autorizadas.</p><div className="quick-status"><div><ArrowUpFromLine size={17} /><span>PC para celular</span><strong>{status.settings.upload ? 'Ligado' : 'Desligado'}</strong></div><div><ArrowDownToLine size={17} /><span>Celular para PC</span><strong>{status.settings.download ? 'Ligado' : 'Desligado'}</strong></div></div><button className={`secondary-button ${status.settings.paused ? 'resume' : ''}`} disabled={busy !== null} onClick={() => { void changeSettings({ paused: !status.settings.paused }); }}>{status.settings.paused ? <Play size={17} /> : <Pause size={17} />}{status.settings.paused ? 'Retomar sincronização' : 'Pausar sincronização'}</button></section></div>
          <section className="panel settings-panel"><div className="panel-title"><div><p className="eyebrow">CONTROLE REMOTO</p><h2>Mouse e teclado</h2></div><MonitorSmartphone size={21} /></div><p>{status.inputActive ? 'Controle ativo no celular conectado.' : status.settings.allowInput ? 'Permitido no PC. Ative no celular para começar.' : 'Desativado. Libere em Preferências para usar o touchpad e o teclado.'}</p><button className="secondary-button" disabled={busy !== null || !status.inputActive} onClick={() => { void runAction('stopInput', desktopApi.stopInput, 'Mouse e teclado desativados. Ative novamente no celular quando precisar.'); }}><Pause size={17} /> Desativar mouse e teclado</button></section>
          {status.pending.length > 0 && <section className="panel pending-strip"><ShieldCheck size={24} /><div><strong>{status.pending.length} pedido{status.pending.length > 1 ? 's' : ''} de acesso aguardando</strong><span>Confira o nome do aparelho antes de confirmar.</span></div><button className="secondary-button" onClick={() => setPage('devices')}>Revisar <ChevronRight size={16} /></button></section>}
        </> : page === 'devices' ? <>
          <div className="page-heading compact"><div><p className="eyebrow">ACESSO AO COMPUTADOR</p><h1>Dispositivos</h1><p>Gerencie os celulares autorizados a controlar este PC.</p></div><button className="primary-button" disabled={busy !== null} onClick={() => { void runAction('pair', desktopApi.startPairing, 'Pareamento aberto por dois minutos.'); setPage('overview'); }}><Plus size={17} /> Adicionar</button></div>
          {status.pending.length > 0 && <section className="panel"><div className="panel-title"><div><p className="eyebrow">CONFIRMAÇÃO NECESSÁRIA</p><h2>Pedidos de pareamento</h2></div></div>{status.pending.map(item => <div className="device-row" key={item.id}><span className="device-icon waiting"><Smartphone size={21} /></span><div><strong>{item.deviceName}</strong><small>Solicita acesso a este computador</small></div><div className="row-actions"><button className="secondary-button danger-text" disabled={busy !== null} onClick={() => { void runAction(item.id, () => desktopApi.approvePairing(item.id, false), 'Pedido recusado.'); }}><ShieldX size={16} /> Recusar</button><button className="primary-button" disabled={busy !== null} onClick={() => { void runAction(item.id, () => desktopApi.approvePairing(item.id, true), 'Celular autorizado.'); }}><Check size={17} /> Permitir</button></div></div>)}</section>}
          <section className="panel"><div className="panel-title"><div><p className="eyebrow">PAREADOS</p><h2>Celulares autorizados</h2></div><span className="count-badge">{status.devices.length}</span></div>{status.devices.length ? status.devices.map(device => <div className="device-row" key={device.id}><span className="device-icon"><Smartphone size={21} /></span><div><strong>{device.name}</strong><small className={device.connected ? 'online-text' : ''}>{device.connected ? 'Conectado agora' : 'Desconectado'}</small></div><button className="text-button danger-text" disabled={busy !== null} onClick={() => { if (window.confirm(`Revogar o acesso de “${device.name}”?`)) void runAction(device.id, () => desktopApi.revokeDevice(device.id), 'Acesso revogado.'); }}>Revogar acesso</button></div>) : <div className="empty-state"><Smartphone size={30} /><strong>Nenhum celular pareado</strong><span>Adicione um celular para começar a controlar este computador.</span></div>}</section>
        </> : page === 'settings' ? <>
          <div className="page-heading compact"><div><p className="eyebrow">NO SEU CONTROLE</p><h1>Preferências</h1><p>As alterações são aplicadas ao serviço do Windows.</p></div></div>
          <section className="panel settings-panel"><div className="panel-title"><div><p className="eyebrow">CONTROLE REMOTO</p><h2>Mouse e teclado</h2></div><MonitorSmartphone size={21} /></div><p>Os cliques e o texto são enviados ao aplicativo em foco no Windows. Selecione a janela desejada antes de digitar. Conteúdo digitado não é salvo nem registrado nos diagnósticos.</p><Toggle checked={status.settings.allowInput} disabled={busy !== null} label="Permitir mouse e teclado pelo celular" detail="Desligado por padrão. Cada conexão também exige ativação no Android; bloquear aqui interrompe o controle imediatamente." onChange={allowInput => { void changeSettings({ allowInput }); }} /><div className="info-note">Aplicativos executados como administrador e a tela protegida do Windows podem bloquear esse controle. Para interromper rapidamente, use Desativar mouse e teclado na bandeja ou na Visão geral.</div></section>
          <section className="panel settings-panel"><div className="panel-title"><div><p className="eyebrow">PRIVACIDADE</p><h2>Área de transferência</h2></div><Clipboard size={21} /></div><p>Sincronização desativada por padrão. O conteúdo não é salvo em histórico e textos sensíveis sinalizados pelo Windows são ignorados.</p>
            <Toggle checked={status.settings.upload} disabled={busy !== null} label="Enviar do PC para o celular" detail="Envia novos textos copiados no Windows enquanto o celular estiver conectado." onChange={upload => { void changeSettings({ upload }); }} />
            <Toggle checked={status.settings.download} disabled={busy !== null} label="Receber do celular no PC" detail="Permite aplicar novos textos enviados pelo Android na área de transferência do Windows." onChange={download => { void changeSettings({ download }); }} />
            <Toggle checked={status.settings.paused} disabled={busy !== null} label="Pausar sincronização" detail="Interrompe rapidamente as duas direções sem perder suas escolhas." onChange={paused => { void changeSettings({ paused }); }} />
          </section><section className="panel settings-panel"><div className="panel-title"><div><p className="eyebrow">SISTEMA</p><h2>Inicialização</h2></div><Power size={21} /></div><Toggle checked={status.settings.autostart} disabled={busy !== null} label="Iniciar com o Windows" detail="Mantém o serviço disponível na bandeja após entrar na sua sessão." onChange={autostart => { void changeSettings({ autostart }); }} /><div className="info-note">Fechar esta janela mantém o serviço na bandeja. Para encerrar, use o menu do ícone na bandeja do Windows.</div></section>
        </> : <>
          <div className="page-heading compact"><div><p className="eyebrow">INFORMAÇÕES DO SERVIÇO</p><h1>Diagnóstico</h1><p>Dados de conexão úteis para parear e resolver problemas.</p></div><button className="secondary-button" onClick={() => { void refresh(); }}><RefreshCw size={16} /> Atualizar</button></div>
          <section className="panel diagnostics-panel"><div className="detail-row"><span>Computador</span><strong>{status.pcName}</strong></div><div className="detail-row"><span>Endereço local</span><strong>{address ? `${address}:${status.port}` : 'Indisponível'}</strong></div><div className="detail-row"><span>ID do PC</span><code>{status.pcId}</code></div><div className="detail-row fingerprint"><span>Identidade do PC (SHA-256)</span><div><code>{status.fingerprint || 'Indisponível'}</code>{status.fingerprint && <button className="icon-button" aria-label="Copiar identidade" onClick={() => { void copyText(status.fingerprint, 'Identidade'); }}><Copy size={16} /></button>}</div></div></section>
          <section className="panel diagnostics-panel"><div className="panel-title"><div><p className="eyebrow">EVENTOS RECENTES</p><h2>Mensagens de diagnóstico</h2></div><RefreshCw size={18} /></div>{status.diagnostics.length ? <ul className="diagnostics-list">{status.diagnostics.map((item, index) => <li key={`${index}-${item}`}>{item}</li>)}</ul> : <div className="empty-state small"><ShieldCheck size={27} /><strong>Nenhum erro recente</strong><span>O serviço está pronto para conexões.</span></div>}<p className="privacy-note"><LockKeyhole size={15} /> Conteúdo copiado e credenciais não aparecem no diagnóstico.</p></section>
        </>}
      </div>
    </main>
  </div>;
}

export default App;
