'use client';

import { useConfig } from '@/contexts/ConfigContext';
import { cn } from '@/lib/utils';

const OPTIONS = [
  { code: 'auto', label: 'Auto' },
  { code: 'en', label: 'EN' },
  { code: 'es', label: 'ES' },
] as const;

/**
 * Compact Auto/EN/ES segmented control that writes the global `selectedLanguage`
 * (persisted + synced to Rust by ConfigContext). Parakeet only auto-detects, so
 * EN/ES are disabled when it's the transcription provider — mirroring the full
 * LanguageSelection dropdown's rule.
 */
export function LanguageQuickPick({ className }: { className?: string }) {
  const { selectedLanguage, setSelectedLanguage, transcriptModelConfig } = useConfig();
  const isParakeet = transcriptModelConfig.provider === 'parakeet';

  // Collapse the 'auto-translate' variant onto 'auto' for this compact control.
  const active = selectedLanguage === 'auto-translate' ? 'auto' : selectedLanguage;

  return (
    <div
      className={cn(
        'inline-flex items-center rounded-md border border-border bg-card p-0.5',
        className
      )}
      role="group"
      aria-label="Transcription language"
    >
      {OPTIONS.map((opt) => {
        const disabled = isParakeet && opt.code !== 'auto';
        const isActive = active === opt.code;
        return (
          <button
            key={opt.code}
            type="button"
            disabled={disabled}
            onClick={() => setSelectedLanguage(opt.code)}
            title={disabled ? 'Parakeet auto-detects the language' : `Transcribe in ${opt.label}`}
            className={cn(
              'rounded px-2 py-0.5 text-xs font-medium transition-colors',
              isActive ? 'bg-accent text-foreground' : 'text-muted-foreground hover:text-foreground',
              disabled && 'cursor-not-allowed opacity-40 hover:text-muted-foreground'
            )}
          >
            {opt.label}
          </button>
        );
      })}
    </div>
  );
}
