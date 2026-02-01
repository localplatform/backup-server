interface Props {
  value: string;
  onChange: (value: string) => void;
}

const presets = [
  { label: 'Every hour', value: '0 * * * *' },
  { label: 'Every 6 hours', value: '0 */6 * * *' },
  { label: 'Daily at midnight', value: '0 0 * * *' },
  { label: 'Daily at 3 AM', value: '0 3 * * *' },
  { label: 'Weekly (Sunday)', value: '0 0 * * 0' },
  { label: 'Monthly (1st)', value: '0 0 1 * *' },
];

export default function CronInput({ value, onChange }: Props) {
  return (
    <div>
      <input
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder="0 * * * * (cron expression)"
      />
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '0.25rem', marginTop: '0.25rem' }}>
        {presets.map(p => (
          <button
            key={p.value}
            type="button"
            className="btn btn-secondary btn-sm"
            style={{ fontSize: '0.7rem', padding: '0.15rem 0.5rem' }}
            onClick={() => onChange(p.value)}
          >
            {p.label}
          </button>
        ))}
      </div>
    </div>
  );
}
