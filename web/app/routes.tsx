import { createBrowserRouter, Navigate } from 'react-router-dom';
import MainLayout from '@/components/layout/MainLayout';
import { NotesPage } from '@/features/daily';
import { OpenCodePage, ClaudePage } from '@/features/coding';
import { SettingsPage } from '@/features/settings';
import { DEFAULT_PATH } from '@/constants';

export const router = createBrowserRouter([
  {
    path: '/',
    element: <MainLayout />,
    children: [
      {
        index: true,
        element: <Navigate to={DEFAULT_PATH} replace />,
      },
      {
        path: 'daily/notes',
        element: <NotesPage />,
      },
      {
        path: 'coding/opencode',
        element: <OpenCodePage />,
      },
      {
        path: 'coding/claude',
        element: <ClaudePage />,
      },
      {
        path: 'settings',
        element: <SettingsPage />,
      },
    ],
  },
]);
