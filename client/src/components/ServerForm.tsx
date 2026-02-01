import { useState } from 'react';

interface Props {
  initial?: { name: string; hostname: string; port: number; ssh_user: string };
  onSubmit: (data: { name: string; hostname: string; port: number; ssh_user: string; password?: string }) => void;
  onCancel: () => void;
  loading?: boolean;
}

export default function ServerForm({ initial, onSubmit, onCancel, loading }: Props) {
  const [name, setName] = useState(initial?.name || '');
  const [hostname, setHostname] = useState(initial?.hostname || '');
  const [port, setPort] = useState(initial?.port || 22);
  const [sshUser, setSshUser] = useState(initial?.ssh_user || '');
  const [password, setPassword] = useState('');

  const isEdit = !!initial;

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    onSubmit({ name, hostname, port, ssh_user: sshUser, ...(!isEdit && password ? { password } : {}) });
  };

  return (
    <form onSubmit={handleSubmit}>
      <div className="form-group">
        <label>Name</label>
        <input value={name} onChange={e => setName(e.target.value)} placeholder="My Server" required />
      </div>
      <div className="form-group">
        <label>Hostname / IP</label>
        <input value={hostname} onChange={e => setHostname(e.target.value)} placeholder="192.168.1.100" required />
      </div>
      <div className="form-group">
        <label>SSH Port</label>
        <input type="number" value={port} onChange={e => setPort(Number(e.target.value))} min={1} max={65535} />
      </div>
      <div className="form-group">
        <label>SSH User</label>
        <input value={sshUser} onChange={e => setSshUser(e.target.value)} placeholder="e.g. admin (user with sudo)" required />
        <small className="form-hint">Un utilisateur avec sudo est requis pour l'installation initiale. Toutes les opérations de backup utiliseront root.</small>
      </div>
      {!isEdit && (
        <div className="form-group">
          <label>SSH Password</label>
          <input type="password" value={password} onChange={e => setPassword(e.target.value)} placeholder="Password for initial setup" required />
        </div>
      )}
      <div className="modal-actions">
        <button type="button" className="btn btn-secondary" onClick={onCancel}>Cancel</button>
        <button type="submit" className="btn" disabled={loading}>
          {loading ? 'Connecting...' : isEdit ? 'Update' : 'Add Server'}
        </button>
      </div>
    </form>
  );
}
