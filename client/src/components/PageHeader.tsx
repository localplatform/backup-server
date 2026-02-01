import './PageHeader.scss';

export interface PageHeaderProps {
  icon?: React.ReactNode;
  title: string;
  stats?: Array<{ label: string; variant?: 'default' | 'running' | 'failed' }>;
  actions?: React.ReactNode;
  leftExtra?: React.ReactNode;
}

export default function PageHeader({ icon, title, stats, actions, leftExtra }: PageHeaderProps) {
  return (
    <div className="page-header-component">
      <div className="header-left">
        {leftExtra}
        <h1>
          {icon}
          {title}
        </h1>
        {stats && stats.length > 0 && (
          <div className="header-stats">
            {stats.map((stat, index) => (
              <span
                key={index}
                className={`stat ${stat.variant === 'running' ? 'stat-running' : stat.variant === 'failed' ? 'stat-failed' : ''}`}
              >
                {stat.label}
              </span>
            ))}
          </div>
        )}
      </div>
      {actions && <div className="header-actions">{actions}</div>}
    </div>
  );
}
