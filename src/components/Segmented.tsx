import { useRef, type KeyboardEvent } from 'react';
import type { LucideIcon } from 'lucide-react';
import { cx } from '../lib/cx';

interface Option<T extends string | number> {
  value: T;
  label: string;
  icon?: LucideIcon;
}

interface SegmentedProps<T extends string | number> {
  label: string;
  value: T;
  options: Option<T>[];
  onChange: (value: T) => void;
}

/** Radio group styled as a segmented control, with arrow-key navigation. */
export function Segmented<T extends string | number>({ label, value, options, onChange }: SegmentedProps<T>) {
  const buttons = useRef<Array<HTMLButtonElement | null>>([]);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1 : event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const index = options.findIndex(option => option.value === value);
    const next = (index + step + options.length) % options.length;
    onChange(options[next].value);
    buttons.current[next]?.focus();
  };

  return (
    <div className="segmented" role="radiogroup" aria-label={label} onKeyDown={onKeyDown}>
      {options.map(({ value: optionValue, label: optionLabel, icon: Icon }, index) => {
        const selected = optionValue === value;
        return (
          <button
            key={String(optionValue)}
            ref={element => {
              buttons.current[index] = element;
            }}
            type="button"
            role="radio"
            aria-checked={selected}
            tabIndex={selected ? 0 : -1}
            className={cx('segment', selected && 'is-selected')}
            onClick={() => onChange(optionValue)}
          >
            {Icon ? <Icon aria-hidden /> : null}
            <span>{optionLabel}</span>
          </button>
        );
      })}
    </div>
  );
}
