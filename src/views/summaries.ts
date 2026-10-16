import type { Settings } from '../types';

/** One sentence describing which clipboard directions are live. */
export function clipboardSummary(settings: Settings): string {
  if (settings.paused) return 'Pausada. Nenhum texto é enviado ou recebido.';
  if (settings.upload && settings.download) return 'Ativa nas duas direções.';
  if (settings.upload) return 'Somente do PC para o celular.';
  if (settings.download) return 'Somente do celular para o PC.';
  return 'Nenhuma direção liberada. Escolha em Transferência.';
}

export function inputSummary(settings: Settings, inputActive: boolean): string {
  if (inputActive) return 'Em uso pelo celular conectado agora.';
  if (settings.allowInput) return 'Liberado. O celular ainda precisa ativar o touchpad.';
  return 'Bloqueado. O celular não pode mover o cursor nem digitar.';
}
