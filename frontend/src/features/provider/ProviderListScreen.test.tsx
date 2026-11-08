import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { QueryClientProvider, QueryClient } from '@tanstack/react-query';
import { RoleProvider } from '../common/RoleContext';
import { ProviderListScreen } from './ProviderListScreen';
import { server } from '../../test/mocks/server';
import { setupServer } from 'msw/node';

// Stub: MSW handler for provider list
beforeAll(() => server.listen());
afterEach(() => server.resetHandlers());
afterAll(() => server.close());

describe('ProviderListScreen', () => {
  it('renders empty state when no providers', async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    render(
      <QueryClientProvider client={queryClient}>
        <RoleProvider current="platform_admin">
          <ProviderListScreen />
        </RoleProvider>
      </QueryClientProvider>,
    );
    // AC-2: empty state present
    expect(await screen.findByText(/provider\.empty/i)).toBeInTheDocument();
  });

  // AC-2: 403 state tested via router, not per screen
  // AC-12: form/create button present in AC-1 design
});
