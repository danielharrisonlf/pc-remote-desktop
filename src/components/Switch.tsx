import { useId, type ReactNode } from 'react';
import type { LucideIcon } from 'lucide-react';
import { cx } from '../lib/cx';

interface SettingSwitchProps {
  label: string;
  description?: ReactNode;
  checked: boolean;
  disabled?: boolean;
  icon?: LucideIcon;
  onChange: (checked: boolean) => void;
}

/** A full-width settings row whose entire surface toggles the switch. */
export function SettingSwitch({ label, description, checked, disabled, icon: Icon, onChange }: SettingSwitchProps) {
  const id = useId();
  return (
    <label className={cx('setting-row', disabled && 'is-disabled')} htmlFor={id}>
      {Icon ? (
        <span className="setting-icon" aria-hidden>
          <Icon />
        </span>
      ) : null}
      <span className="setting-text">
        <span className="setting-label">{label}</span>
        {description ? <span className="setting-description" id={`${id}-description`}>{description}</span> : null}
      </span>
      <span className="switch">
        <input
          id={id}
          type="checkbox"
          role="switch"
          checked={checked}
          disabled={disabled}
          aria-describedby={description ? `${id}-description` : undefined}
          onChange={event => onChange(event.target.checked)}
        />
        <span className="switch-track" aria-hidden>
          <span className="switch-thumb" />
        </span>
      </span>
    </label>
  );
}
