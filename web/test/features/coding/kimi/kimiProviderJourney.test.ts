import test from 'node:test';
import assert from 'node:assert/strict';

import type {
  KimiProvider,
  KimiProviderFormData,
} from '../../../../types/kimi.ts';
import {
  KIMI_LOCAL_PROVIDER_ID,
} from '../../../../types/kimi.ts';
import {
  buildKimiProviderSavePlan,
  shouldReengageKimiGatewayOnSave,
} from '../../../../features/coding/kimi/utils/providerSaveFlow.ts';

/**
 * Minimal in-memory mirror of the backend provider endpoints, enough to walk
 * the real user journey from a fresh install: the DB is empty, so
 * `list_kimi_providers` projects the on-disk config.toml as the `__local__`
 * provider; adopting it persists a real applied record and the projection
 * disappears.
 */
class InMemoryKimiProviderStore {
  providers: KimiProvider[] = [];
  /** Simulates `credentials/*.json` existing on disk for the `__local__` projection. */
  hasLocalCredentials = false;
  localCategory: 'official' | 'custom' = 'official';
  private nextId = 1;

  list(): KimiProvider[] {
    if (this.providers.length > 0) {
      return this.providers.map((provider) => ({ ...provider }));
    }
    // Backend projection: only when the DB is empty and a config.toml exists.
    if (this.localCategory === 'official' && !this.hasLocalCredentials) {
      return [];
    }
    const now = '2026-01-01T00:00:00Z';
    return [{
      id: KIMI_LOCAL_PROVIDER_ID,
      name: 'Local Kimi',
      category: this.localCategory,
      settingsConfig: JSON.stringify({ auth: {}, defaultModelKey: 'kimi-code/k3' }),
      isApplied: true,
      isDisabled: false,
      createdAt: now,
      updatedAt: now,
    }];
  }

  appliedId(): string {
    return this.providers.find((provider) => provider.isApplied)?.id ?? '';
  }

  adoptLocal(values: KimiProviderFormData): KimiProvider {
    assert.equal(this.providers.length, 0, 'adopt_local only valid while DB is empty');
    const now = '2026-01-01T00:00:01Z';
    const created: KimiProvider = {
      id: `p-${this.nextId++}`,
      name: values.name,
      category: values.category,
      settingsConfig: values.settingsConfig,
      notes: values.notes,
      meta: values.meta,
      // save_kimi_local_config marks the adopted record applied: the on-disk
      // config keeps working, it is now just managed.
      isApplied: true,
      isDisabled: false,
      createdAt: now,
      updatedAt: now,
    };
    this.providers.push(created);
    return created;
  }

  update(provider: KimiProvider): KimiProvider {
    const index = this.providers.findIndex((entry) => entry.id === provider.id);
    assert.ok(index >= 0, `provider ${provider.id} must exist for update`);
    this.providers[index] = { ...provider };
    return this.providers[index];
  }

  create(values: KimiProviderFormData): KimiProvider {
    const now = '2026-01-01T00:00:02Z';
    const created: KimiProvider = {
      id: `p-${this.nextId++}`,
      name: values.name,
      category: values.category,
      settingsConfig: values.settingsConfig,
      notes: values.notes,
      meta: values.meta,
      isApplied: false,
      isDisabled: false,
      createdAt: now,
      updatedAt: now,
    };
    this.providers.push(created);
    return created;
  }

  apply(id: string): void {
    for (const provider of this.providers) {
      provider.isApplied = provider.id === id;
    }
  }

  remove(id: string): void {
    if (this.providers.find((provider) => provider.id === id)?.isApplied) {
      throw new Error('The applied Kimi provider cannot be deleted');
    }
    this.providers = this.providers.filter((provider) => provider.id !== id);
  }
}

const localConfigValues: KimiProviderFormData = {
  name: 'My Kimi',
  category: 'custom',
  settingsConfig: JSON.stringify({
    auth: { API_KEY: 'sk-test' },
    defaultModelKey: 'relay/k3',
    providerConfigs: { relay: { type: 'openai', base_url: 'https://relay.example.com/v1' } },
    modelCatalog: { models: [{ key: 'relay/k3', model: 'k3', provider: 'relay' }] },
  }),
};

test('journey: fresh install with official login projects __local__ as official', () => {
  const store = new InMemoryKimiProviderStore();
  store.localCategory = 'official';
  store.hasLocalCredentials = true;

  const providers = store.list();
  assert.equal(providers.length, 1);
  assert.equal(providers[0].id, KIMI_LOCAL_PROVIDER_ID);
  assert.equal(providers[0].category, 'official', 'official credentials must not be misread as custom');
  assert.equal(providers[0].isApplied, true);
});

test('journey: adopting the local config does not duplicate it', () => {
  const store = new InMemoryKimiProviderStore();
  store.localCategory = 'custom';
  let providers = store.list();
  assert.equal(providers.length, 1);
  const localProvider = providers[0];
  assert.equal(localProvider.id, KIMI_LOCAL_PROVIDER_ID);

  const plan = buildKimiProviderSavePlan(localProvider, localConfigValues);
  assert.equal(plan.action, 'adopt_local', 'editing __local__ must adopt, not create');

  // Execute the plan the way KimiPage does for adopt_local.
  const saved = store.adoptLocal(localConfigValues);
  assert.equal(saved.isApplied, true, 'adopted local config stays applied');

  providers = store.list();
  assert.equal(providers.length, 1, 'projection must disappear after adoption, not leave two rows');
  assert.notEqual(providers[0].id, KIMI_LOCAL_PROVIDER_ID);
  assert.equal(store.appliedId(), providers[0].id);
});

test('journey: re-editing an adopted provider updates in place', () => {
  const store = new InMemoryKimiProviderStore();
  store.localCategory = 'custom';
  const adopted = store.adoptLocal(localConfigValues);

  const edited = buildKimiProviderSavePlan(adopted, {
    ...localConfigValues,
    name: 'My Kimi Renamed',
    settingsConfig: JSON.stringify({ auth: { API_KEY: 'sk-new' } }),
  });
  assert.equal(edited.action, 'update', 'a real record must update, never create');

  store.update(edited.provider);
  const providers = store.list();
  assert.equal(providers.length, 1, 'editing must not add a second row');
  assert.equal(providers[0].id, adopted.id, 'same record id preserved');
  assert.equal(providers[0].name, 'My Kimi Renamed');
  assert.equal(providers[0].isApplied, true, 'applied flag preserved through edit');
});

test('journey: adding a second provider appends without touching the applied one', () => {
  const store = new InMemoryKimiProviderStore();
  store.localCategory = 'custom';
  const adopted = store.adoptLocal(localConfigValues);

  const plan = buildKimiProviderSavePlan(null, localConfigValues);
  assert.equal(plan.action, 'create');
  const created = store.create(plan.input);

  const providers = store.list();
  assert.equal(providers.length, 2);
  assert.equal(created.isApplied, false, 'new provider starts unapplied');
  assert.equal(store.appliedId(), adopted.id);
});

test('journey: applying another provider moves the applied flag', () => {
  const store = new InMemoryKimiProviderStore();
  store.localCategory = 'custom';
  store.adoptLocal(localConfigValues);
  const second = store.create({ ...localConfigValues, name: 'Second' });

  store.apply(second.id);
  assert.equal(store.appliedId(), second.id);

  const providers = store.list();
  assert.equal(providers.find((provider) => provider.id !== second.id)?.isApplied, false);
});

test('journey: deletion protects the applied provider and permits removing an unused one', () => {
  const store = new InMemoryKimiProviderStore();
  store.localCategory = 'custom';
  const adopted = store.adoptLocal(localConfigValues);
  const second = store.create({ ...localConfigValues, name: 'Second' });
  store.apply(second.id);

  assert.throws(() => store.remove(second.id), /applied Kimi provider cannot be deleted/);
  store.remove(adopted.id);
  const providers = store.list();
  assert.equal(providers.length, 1);
  assert.equal(providers[0].id, second.id);
  assert.equal(store.appliedId(), second.id);
});

test('journey: editing with a cleared meta must not resurrect stale billing headers', () => {
  const store = new InMemoryKimiProviderStore();
  store.localCategory = 'custom';
  const adopted = store.adoptLocal({
    ...localConfigValues,
    meta: { costMultiplier: '1.5', customHeaders: [{ op: 'set', name: 'X-Test', value: '1' }] },
  });
  assert.ok(adopted.meta);

  // User clears both sections -> merge utils return undefined meta.
  const edited = buildKimiProviderSavePlan(adopted, { ...localConfigValues, meta: undefined });
  assert.equal(edited.action, 'update');
  const updated = store.update({ ...edited.provider, meta: undefined });
  assert.equal(updated.meta, undefined);
});

test('journey: gateway re-engage for any save that rewrites live files', () => {
  const store = new InMemoryKimiProviderStore();
  store.localCategory = 'custom';
  const adopted = store.adoptLocal(localConfigValues);
  const localProjection = store.list().find((provider) => provider.id === KIMI_LOCAL_PROVIDER_ID)
    ?? { ...adopted, id: KIMI_LOCAL_PROVIDER_ID, isApplied: true };

  assert.equal(shouldReengageKimiGatewayOnSave(localProjection, 'single'), true,
    'adoption rewrites the live config.toml, so the backend gate rejects it during takeover');
  assert.equal(shouldReengageKimiGatewayOnSave(localProjection, 'failover'), true,
    'adoption must restore direct first under a failover takeover too');
  assert.equal(shouldReengageKimiGatewayOnSave(adopted, 'single'), true);
  assert.equal(shouldReengageKimiGatewayOnSave(adopted, 'failover'), true);
  assert.equal(shouldReengageKimiGatewayOnSave(adopted, null), false);

  const unapplied = store.create(localConfigValues);
  assert.equal(shouldReengageKimiGatewayOnSave(unapplied, 'single'), false);
  assert.equal(shouldReengageKimiGatewayOnSave(unapplied, 'failover'), false,
    'unapplied saves only touch the DB row; failover stays untouched');
});

test('journey: starting a login writes no provider row', () => {
  const store = new InMemoryKimiProviderStore();
  store.localCategory = 'official';
  store.hasLocalCredentials = true;

  // The frontend used to create the official provider *before* the device flow
  // started, so every cancelled login left an empty `Kimi Official` card behind.
  // The row is now created by the backend once the exchange succeeds, which
  // means starting a login must touch nothing.
  const officialInList = () => store.providers.find((provider) => provider.category === 'official');
  assert.equal(officialInList(), undefined);
  assert.equal(store.providers.length, 0, 'a started login creates no rows');

  // The row appears only when the login completes, and a second login reuses it
  // rather than adding another.
  const created = store.create({
    name: 'Kimi Official',
    category: 'official',
    settingsConfig: JSON.stringify({ auth: { API_KEY: '' }, defaultModelKey: 'kimi-code/k3', providerConfigs: {} }),
  });
  assert.equal(created.category, 'official');
  assert.equal(officialInList()?.id, created.id);
});

test('journey: copying a provider always creates, never updates or adopts', () => {
  const store = new InMemoryKimiProviderStore();
  store.localCategory = 'custom';
  const adopted = store.adoptLocal(localConfigValues);

  // Copy of a real record: the cloned editingProvider carries the source id,
  // but isCopy must force a create so the source row is never overwritten.
  const copiedPlan = buildKimiProviderSavePlan(
    { ...adopted, id: `${adopted.id}_copy`, name: `${adopted.name}_copy`, isApplied: false },
    localConfigValues,
    { isCopy: true },
  );
  assert.equal(copiedPlan.action, 'create', 'copy of an applied row must create a new record');

  // Copy of the __local__ projection must not re-adopt either — adoption only
  // applies to a direct edit of the placeholder, never to a clone.
  store.providers = [];
  const localProjection = store.list()[0];
  const copiedLocalPlan = buildKimiProviderSavePlan(localProjection, localConfigValues, { isCopy: true });
  assert.equal(copiedLocalPlan.action, 'create', 'copy of __local__ must create, not adopt');
});
