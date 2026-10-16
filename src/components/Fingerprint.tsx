import { fingerprintGroups, fingerprintShort, fingerprintSwatches } from '../lib/fingerprint';
import { CopyButton } from './CopyButton';

/** Eight color blocks derived from the digest, shown only while pairing. The Android app draws the same strip. */
export function FingerprintStrip({ value }: { value: string }) {
  const swatches = fingerprintSwatches(value);
  if (!swatches.length) return null;
  return (
    <span className="fp-strip" role="img" aria-label="Assinatura visual da identidade">
      {swatches.map((color, index) => (
        <span key={index} style={{ background: color }} />
      ))}
    </span>
  );
}

export function FingerprintInline({ value }: { value: string }) {
  if (!value) return <span className="muted">Indisponível</span>;
  return <code className="fp-inline mono">{fingerprintShort(value)}</code>;
}

/** Full SHA-256 as 16 hex groups with a copy action. */
export function FingerprintPanel({ value }: { value: string }) {
  const groups = fingerprintGroups(value);
  if (!groups.length) return <p className="muted">A identidade do PC ainda não foi carregada.</p>;
  return (
    <div className="fp-panel">
      <div className="fp-grid mono" aria-label={`SHA-256 ${groups.join(' ')}`}>
        {groups.map((group, index) => (
          <span key={index}>{group}</span>
        ))}
      </div>
      <div className="fp-footer">
        <CopyButton value={value} label="identidade SHA-256" text="Copiar SHA-256" />
      </div>
    </div>
  );
}
