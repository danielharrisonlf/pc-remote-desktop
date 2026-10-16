import type { Settings } from '../types';

export function clipboardSummary(settings: Settings): string {
  if (settings.paused) return 'Pausada';
  if (settings.upload && settings.download) return 'Nos dois sentidos';
  if (settings.upload) return 'Só do PC para o celular';
  if (settings.download) return 'Só do celular para o PC';
  return 'Nenhum sentido ativo';
}

export function inputSummary(settings: Settings, inputActive: boolean): string {
  if (inputActive) return 'Em uso';
  if (settings.allowInput) return 'Liberado';
  return 'Bloqueado';
}
