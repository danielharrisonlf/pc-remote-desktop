import { forwardRef, type ButtonHTMLAttributes } from 'react';
import { LoaderCircle, type LucideIcon } from 'lucide-react';
import { cx } from '../lib/cx';

type Variant = 'primary' | 'secondary' | 'ghost' | 'danger' | 'danger-ghost';

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: 'sm' | 'md' | 'lg';
  icon?: LucideIcon;
  loading?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = 'secondary', size = 'md', icon: Icon, loading = false, disabled, className, children, type = 'button', ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      className={cx('btn', `btn-${variant}`, `btn-${size}`, loading && 'is-loading', className)}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      {...rest}
    >
      {loading ? <LoaderCircle className="btn-spinner" aria-hidden /> : Icon ? <Icon aria-hidden /> : null}
      {children ? <span className="btn-label">{children}</span> : null}
    </button>
  );
});

interface IconButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'children'> {
  label: string;
  icon: LucideIcon;
  variant?: 'ghost' | 'secondary';
  size?: 'sm' | 'md';
}

export function IconButton({ label, icon: Icon, variant = 'ghost', size = 'md', className, type = 'button', ...rest }: IconButtonProps) {
  return (
    <button type={type} className={cx('icon-btn', `icon-btn-${variant}`, `icon-btn-${size}`, className)} aria-label={label} title={label} {...rest}>
      <Icon aria-hidden />
    </button>
  );
}
