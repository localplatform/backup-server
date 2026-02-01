import './StatusBadge.scss';

interface Props {
  status: string;
  size?: 'sm' | 'md';
}

const statusColors: Record<string, string> = {
  connected: 'success',
  key_registered: 'info',
  key_generated: 'warning',
  pending: 'muted',
  error: 'danger',
  idle: 'muted',
  running: 'info',
  completed: 'success',
  failed: 'danger',
  cancelled: 'warning',
};

export default function StatusBadge({ status, size = 'md' }: Props) {
  const color = statusColors[status] || 'muted';
  return (
    <span className={`status-badge ${color} ${size}`}>
      {status.replace(/_/g, ' ')}
    </span>
  );
}
