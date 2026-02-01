# Backup Server

Monorepo npm workspaces : `server/` (Node.js/Express/TypeScript) + `client/` (React/Vite/SASS).
Deux processus DEV via PM2 : back (tsx watch, port 3000) + front (Vite dev server, port 5173 avec proxy vers le back).

## Build

```bash
npm install                # Installer toutes les dépendances
npm run build              # Build complet (client vite + server tsup)
npm run build:client       # Build client seul
npm run build:server       # Build server seul
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

## Vérifier que les services répondent

```bash
curl -s localhost:3000/api/servers   # API back
curl -s localhost:5173               # Vite dev server
```

## Persistance de la base de données

SQLite est configuré en mode `DELETE` (journal classique) plutôt que WAL pour garantir la durabilité en développement :

- **Mode journal DELETE :** Chaque transaction est écrite immédiatement dans le fichier `.db` (pas de fichier WAL/SHM)
- **Synchronous FULL :** Garanties ACID strictes, chaque écriture attend la confirmation du disque
- **Shutdown handler amélioré :** Flush explicite de la DB avant fermeture du processus
- **Timeout PM2 étendu :** 10 secondes pour permettre un shutdown gracieux complet
- **Backups automatiques :** Copie quotidienne au démarrage dans `server/data/backups/` (7 jours de rétention)

**Pourquoi ce mode ?** En développement avec tsx watch, le serveur redémarre fréquemment. Le mode WAL (Write-Ahead Logging) peut perdre des transactions si le processus est tué brutalement. Le mode DELETE sacrifie un peu de performance pour une durabilité maximale.

**Note production :** Après `npm run build`, le mode WAL peut être réactivé dans `server/src/db/connection.ts` pour de meilleures performances si les redémarrages sont rares.

## Architecture

- **Server :** Express REST API + WebSocket sur port 3000. SQLite (better-sqlite3) pour la persistance. SSH2 pour les connexions distantes. Rsync pour les transferts.
- **Client :** React 18 + React Router + TanStack React Query + SASS. Vite dev server (port 5173) avec proxy `/api` et `/ws` vers le back (port 3000). Zéro polling — toutes les mises à jour via WebSocket.

## Répertoires clés

- `server/data/` — Données runtime (SQLite DB, clés SSH). Gitignored.
- `server/src/services/` — Logique métier (SSH, rsync, scheduling)
- `server/src/routes/` — Routes API Express
- `client/src/hooks/` — Hooks React (WebSocket, data fetching)
- `client/src/pages/` — Composants page
- `client/src/components/` — Composants UI réutilisables
