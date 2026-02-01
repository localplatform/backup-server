import { z } from 'zod';
import { v4 as uuidv4 } from 'uuid';
import { getDb } from '../db/connection.js';

export const SshStatus = z.enum(['pending', 'key_generated', 'key_registered', 'connected', 'error']);
export type SshStatus = z.infer<typeof SshStatus>;

export const ServerSchema = z.object({
  id: z.string().uuid(),
  name: z.string().min(1),
  hostname: z.string().min(1),
  port: z.number().int().min(1).max(65535).default(22),
  ssh_user: z.string().min(1),
  ssh_key_path: z.string().nullable().default(null),
  ssh_status: SshStatus.default('pending'),
  ssh_error: z.string().nullable().default(null),
  rsync_installed: z.number().int().default(0),
  use_sudo: z.number().int().default(0),
  last_seen_at: z.string().nullable().default(null),
  created_at: z.string(),
  updated_at: z.string(),
});

export type Server = z.infer<typeof ServerSchema>;

export const CreateServerSchema = z.object({
  name: z.string().min(1),
  hostname: z.string().min(1),
  port: z.number().int().min(1).max(65535).default(22),
  ssh_user: z.string().min(1),
  password: z.string().min(1),
});

export const UpdateServerSchema = z.object({
  name: z.string().min(1).optional(),
  hostname: z.string().min(1).optional(),
  port: z.number().int().min(1).max(65535).optional(),
  ssh_user: z.string().min(1).optional(),
});

export type CreateServer = z.infer<typeof CreateServerSchema>;
export type CreateServerData = Omit<CreateServer, 'password'>;
export type UpdateServer = z.infer<typeof UpdateServerSchema>;

export const serverModel = {
  findAll(): Server[] {
    return getDb().prepare('SELECT * FROM source_servers ORDER BY created_at DESC').all() as Server[];
  },

  findById(id: string): Server | undefined {
    return getDb().prepare('SELECT * FROM source_servers WHERE id = ?').get(id) as Server | undefined;
  },

  create(data: CreateServerData): Server {
    const id = uuidv4();
    const now = new Date().toISOString();
    getDb().prepare(
      `INSERT INTO source_servers (id, name, hostname, port, ssh_user, created_at, updated_at)
       VALUES (?, ?, ?, ?, ?, ?, ?)`
    ).run(id, data.name, data.hostname, data.port, data.ssh_user, now, now);
    return this.findById(id)!;
  },

  update(id: string, data: Partial<Server>): Server | undefined {
    const existing = this.findById(id);
    if (!existing) return undefined;

    const fields: string[] = [];
    const values: unknown[] = [];
    for (const [key, value] of Object.entries(data)) {
      if (key === 'id' || key === 'created_at') continue;
      fields.push(`${key} = ?`);
      values.push(value);
    }
    fields.push("updated_at = datetime('now')");
    values.push(id);

    getDb().prepare(`UPDATE source_servers SET ${fields.join(', ')} WHERE id = ?`).run(...values);
    return this.findById(id)!;
  },

  delete(id: string): boolean {
    const result = getDb().prepare('DELETE FROM source_servers WHERE id = ?').run(id);
    return result.changes > 0;
  },
};
