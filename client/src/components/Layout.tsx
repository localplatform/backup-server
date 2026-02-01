import { NavLink, Outlet } from 'react-router-dom';
import { Server, HardDrive, LayoutDashboard, FolderSync, Database } from 'lucide-react';
import { useWebSocket } from '../hooks/useWebSocket.js';
import './Layout.scss';

export default function Layout() {
  const { connected } = useWebSocket();

  return (
    <div className="layout">
      <nav className="sidebar">
        <div className="sidebar-header">
          <HardDrive size={24} />
          <span>Backup Server</span>
        </div>
        <ul className="sidebar-nav">
          <li>
            <NavLink to="/" end>
              <LayoutDashboard size={18} />
              Dashboard
            </NavLink>
          </li>
          <li>
            <NavLink to="/servers">
              <Server size={18} />
              Servers
            </NavLink>
          </li>
          <li>
            <NavLink to="/jobs">
              <FolderSync size={18} />
              Backup Jobs
            </NavLink>
          </li>
          <li>
            <NavLink to="/storage">
              <Database size={18} />
              Storage
            </NavLink>
          </li>
        </ul>
        <div className={`ws-status ${connected ? 'connected' : 'disconnected'}`}>
          <span className="ws-dot" />
          {connected ? 'Connected' : 'Disconnected'}
        </div>
      </nav>
      <main className="main-content">
        <Outlet />
      </main>
    </div>
  );
}
