import { useEffect, useRef, useState } from 'react';
import { RefreshCw, ShieldCheck } from 'lucide-react';
import { desktopApi } from '../api';
import { errorMessage, type NetworkAccessStatus } from '../types';
import { Button } from './Button';

export function NetworkAccessPanel({ compact = false, card = false, onStatus }: {
  compact?: boolean;
  card?: boolean;
  onStatus?: (status: NetworkAccessStatus) => void;
}) {
  const [status, setStatus] = useState<NetworkAccessStatus | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState<'check' | 'repair' | null>('check');
  const mounted = useRef(false);
  const reportStatus = useRef(onStatus);
  reportStatus.current = onStatus;

  useEffect(() => {
    let active = true;
    mounted.current = true;
    desktopApi.networkAccessStatus().then(value => {
      if (!active) return;
      setStatus(value);
      reportStatus.current?.(value);
    }).catch(reason => {
      if (active) setError(errorMessage(reason));
    }).finally(() => {
      if (active) setBusy(null);
    });
    return () => { active = false; mounted.current = false; };
  }, []);

  const execute = async (repair: boolean) => {
    setBusy(repair ? 'repair' : 'check');
    setError('');
    try {
      const value = await (repair ? desktopApi.repairNetworkAccess() : desktopApi.networkAccessStatus());
      if (!mounted.current) return;
      setStatus(value);
      reportStatus.current?.(value);
    } catch (reason) {
      if (mounted.current) setError(errorMessage(reason));
    } finally {
      if (mounted.current) setBusy(null);
    }
  };

  if (status?.supported === false) return null;
  const Container = card ? 'section' : 'div';
  const Heading = compact ? 'h3' : 'h2';
  return (
    <Container className={`${card ? 'card ' : ''}network-access${compact ? ' is-compact' : ''}`}>
      <Heading className="card-title">Conexões na rede local</Heading>
      <p className="muted" role="status" aria-live="polite">
        {error || status?.message || 'Conferindo as permissões de rede…'}
      </p>
      {status?.state !== 'configured' ? (
        <>
          <p className="card-description">Para permitir este app na rede local, o Windows pedirá permissão de administrador.</p>
          <div className="network-access-actions">
            <Button icon={ShieldCheck} disabled={busy !== null} loading={busy === 'repair'} onClick={() => void execute(true)}>
              Permitir conexões na rede
            </Button>
            <Button variant="ghost" size="sm" icon={RefreshCw} disabled={busy !== null} loading={busy === 'check'} onClick={() => void execute(false)}>
              Verificar
            </Button>
          </div>
        </>
      ) : (
        <Button variant="ghost" size="sm" icon={RefreshCw} disabled={busy !== null} loading={busy === 'check'} onClick={() => void execute(false)}>
          Verificar novamente
        </Button>
      )}
    </Container>
  );
}
