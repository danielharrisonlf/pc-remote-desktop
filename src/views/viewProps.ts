import type { ServiceState } from '../hooks/useStatus';
import type { Page } from '../navigation';
import type { DesktopStatus, Settings } from '../types';

export type RunAction = (key: string, action: () => Promise<unknown>, success?: string) => Promise<boolean>;

export interface ViewProps {
  status: DesktopStatus;
  /** Settings including changes still being saved. */
  settings: Settings;
  changeSettings: (patch: Partial<Settings>, success?: string) => void;
  run: RunAction;
  isBusy: (key: string) => boolean;
  serviceState: ServiceState;
  now: number;
  navigate: (page: Page) => void;
  openPairing: () => void;
}
