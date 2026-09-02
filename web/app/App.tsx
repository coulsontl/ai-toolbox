import React from 'react';
import { Spin } from 'antd';
import { RouterProvider } from 'react-router-dom';
import { router } from './routes';
import { Providers } from './providers';
import { getStartupRecovery } from '@/services';
import RecoveryApp from './RecoveryApp';

type RecoveryState =
  | { status: 'loading' }
  | { status: 'recovery'; message: string }
  | { status: 'normal' };

function App() {
  const [state, setState] = React.useState<RecoveryState>({ status: 'loading' });

  React.useEffect(() => {
    let cancelled = false;
    getStartupRecovery()
      .then((message) => {
        if (cancelled) return;
        setState(message ? { status: 'recovery', message } : { status: 'normal' });
      })
      .catch((error) => {
        // If the recovery command itself is unavailable, fall back to the
        // normal boot path (which will surface any real DB error its own way).
        console.error('Failed to query startup recovery state:', error);
        if (!cancelled) setState({ status: 'normal' });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (state.status === 'loading') {
    return (
      <div
        style={{
          display: 'flex',
          justifyContent: 'center',
          alignItems: 'center',
          minHeight: '100vh',
        }}
      >
        <Spin size="large" />
      </div>
    );
  }

  if (state.status === 'recovery') {
    return <RecoveryApp errorMessage={state.message} />;
  }

  return (
    <Providers>
      <RouterProvider router={router} />
    </Providers>
  );
}

export default App;
