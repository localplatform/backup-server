import { BrowserRouter, Routes, Route } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { Toaster } from 'react-hot-toast';
import Layout from './components/Layout.js';
import Dashboard from './pages/Dashboard.js';
import Servers from './pages/Servers.js';
import ServerDetail from './pages/ServerDetail.js';
import BackupJobs from './pages/BackupJobs.js';
import Storage from './pages/Storage.js';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: Infinity, // WS handles updates
      refetchOnWindowFocus: false,
    },
  },
});

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <Routes>
          <Route element={<Layout />}>
            <Route path="/" element={<Dashboard />} />
            <Route path="/servers" element={<Servers />} />
            <Route path="/servers/:id" element={<ServerDetail />} />
            <Route path="/jobs" element={<BackupJobs />} />
            <Route path="/storage" element={<Storage />} />
          </Route>
        </Routes>
      </BrowserRouter>
      <Toaster
        position="bottom-right"
        toastOptions={{
          style: {
            background: '#1e293b',
            color: '#f1f5f9',
            border: '1px solid #334155',
          },
        }}
      />
    </QueryClientProvider>
  );
}
