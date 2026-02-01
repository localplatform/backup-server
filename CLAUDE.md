# Backup Server

Monorepo : `backup-server-rs/` (Rust/Axum backend, port 3000) + `client/` (React/Vite/SASS, port 5173 avec proxy vers le back).
Deux processus via PM2 : back (binaire Rust compilé) + front (Vite dev server).

## IMPORTANT — Build avant de tester

**Toujours builder le backend Rust ET le frontend avant de demander de tester :**

```bash
cargo build --release -p backup-server-rs   # Backend Rust
npm run build:client                        # Frontend React
```

Ou en une seule commande :

```bash
npm run build                               # Build complet (server + client)
```

Le binaire compilé se trouve à `target/release/backup-server` (workspace target à la racine). PM2 lance ce binaire directement. Le frontend compilé se trouve dans `client/dist/` et est servi en statique par le backend Rust.

## Build

```bash
npm install                # Installer les dépendances client
npm run build              # Build complet (server Rust + client Vite)
npm run build:client       # Build client seul
npm run build:server       # Build server Rust seul (cargo build --release)
```

## Gestion des processus (PM2)

Le back et le front tournent via PM2 (`ecosystem.config.cjs`).
PM2 est configuré pour le **démarrage automatique au reboot** (`pm2-root` systemd unit).

```bash
pm2 list                   # État des processus
pm2 start ecosystem.config.cjs  # Démarrer les deux processus
pm2 restart all            # Redémarrer tout
pm2 restart backup-server  # Redémarrer le back seul
pm2 restart backup-client  # Redémarrer le front seul
pm2 stop all               # Arrêter tout
pm2 logs                   # Suivre tous les logs en temps réel
pm2 logs backup-server     # Logs du back uniquement
pm2 logs backup-client     # Logs du front uniquement
pm2 save                   # Sauvegarder la liste pour le reboot
```

## Variables d'environnement (backend Rust)

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `3000` | Port HTTP du serveur |
| `BACKUPS_DIR` | `/backup/data/backups` | Répertoire racine des backups |
| `RUST_LOG` | `info` | Niveau de log (trace, debug, info, warn, error) |
| `MAX_CONCURRENT_GLOBAL` | `8` | Max backups simultanés global |
| `MAX_CONCURRENT_PER_SERVER` | `4` | Max backups simultanés par serveur |
| `BACKUP_SERVER_IP` | *(auto-détecté)* | IP du serveur accessible par les agents |

## Vérifier que les services répondent

```bash
curl -s localhost:3000/api/servers   # API back
curl -s localhost:5173               # Vite dev server
```

## Persistance de la base de données

SQLite via `rusqlite` (bundled) avec pool `r2d2_sqlite` :

- **Mode journal DELETE** : durabilité maximale, pas de fichier WAL/SHM
- **Synchronous FULL** : garanties ACID strictes
- **Backups automatiques** : copie quotidienne au démarrage dans `server/data/backups/` (7 jours de rétention)
- **Fichier DB** : `server/data/backup-server.db` (identique à l'ancien backend)

## Architecture

- **Backend** : Rust/Axum sur port 3000. SQLite (rusqlite + r2d2) pour la persistance. WebSocket UI (`/ws`) pour le broadcast temps réel. WebSocket Agent (`/ws/agent`) pour la communication avec les agents de backup. SSH2 pour le déploiement d'agents.
- **Client** : React 18 + React Router + TanStack React Query + SASS. Vite dev server (port 5173) avec proxy `/api` et `/ws` vers le back (port 3000). Zéro polling — toutes les mises à jour via WebSocket.
- **Agent** : Binaire Rust (`backup-agent/`) déployé sur les serveurs distants. Se connecte au serveur via WebSocket reverse.

## Cargo Workspace

Le workspace Cargo (`Cargo.toml` racine) contient deux crates :
- `backup-server-rs/` — Le backend HTTP/WebSocket
- `backup-agent/` — L'agent déployé sur les machines distantes

```bash
cargo build --release -p backup-server-rs   # Build le serveur
cargo build --release -p backup-agent       # Build l'agent
cargo check                                 # Vérifier la compilation
```

## Répertoires clés

- `backup-server-rs/src/` — Code source du backend Rust
  - `routes/` — Routes API Axum (servers, jobs, versions, storage, files, agent, explorer)
  - `services/` — Logique métier (orchestrateur, scheduler, déploiement, ping)
  - `models/` — Modèles de données et requêtes SQLite
  - `ws/` — WebSocket UI (broadcast) et Agent (registry)
  - `db/` — Connexion pool et migrations
- `backup-agent/src/` — Code source de l'agent
- `client/src/` — Code source React
  - `hooks/` — Hooks React (WebSocket, data fetching)
  - `pages/` — Composants page
  - `components/` — Composants UI réutilisables
- `server/data/` — Données runtime (SQLite DB, clés SSH). Gitignored.
