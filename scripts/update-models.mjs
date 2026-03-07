#!/usr/bin/env node

/**
 * Fetch models.dev.json from https://models.dev/api.json and replace tauri/resources/models.dev.json
 * Validates structure before replacing to avoid breaking the build.
 */

import { writeFileSync, readFileSync } from "fs";
import { resolve, dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const API_URL = "https://models.dev/api.json";
const TARGET_PATH = resolve(__dirname, "../tauri/resources/models.dev.json");

async function fetchModels() {
  console.log(`Fetching models from ${API_URL} ...`);
  const res = await fetch(API_URL);
  if (!res.ok) {
    throw new Error(`HTTP ${res.status}: ${res.statusText}`);
  }
  const raw = await res.text();
  const data = JSON.parse(raw);
  return { raw, data };
}

/**
 * Validate that the fetched JSON matches the expected structure.
 * Required by tauri/src/coding/open_code/free_models.rs:
 *   - Root must be a non-empty object (provider_id -> provider_data)
 *   - Each provider must have "name" (string) and "models" (object)
 *   - Each model must be an object with "name" (string)
 */
function validate(data) {
  const errors = [];

  if (typeof data !== "object" || data === null || Array.isArray(data)) {
    errors.push("Root is not a JSON object");
    return errors;
  }

  const providerIds = Object.keys(data);
  if (providerIds.length === 0) {
    errors.push("Root object has no providers");
    return errors;
  }

  let validatedProviders = 0;
  let validatedModels = 0;

  for (const providerId of providerIds) {
    const provider = data[providerId];
    const prefix = `provider "${providerId}"`;

    if (typeof provider !== "object" || provider === null || Array.isArray(provider)) {
      errors.push(`${prefix}: not an object`);
      continue;
    }

    if (typeof provider.name !== "string" || provider.name.length === 0) {
      errors.push(`${prefix}: missing or invalid "name"`);
    }

    if (typeof provider.models !== "object" || provider.models === null || Array.isArray(provider.models)) {
      errors.push(`${prefix}: missing or invalid "models" object`);
      continue;
    }

    const modelIds = Object.keys(provider.models);
    for (const modelId of modelIds) {
      const model = provider.models[modelId];
      const mPrefix = `${prefix} > model "${modelId}"`;

      if (typeof model !== "object" || model === null || Array.isArray(model)) {
        errors.push(`${mPrefix}: not an object`);
        continue;
      }

      if (typeof model.name !== "string" || model.name.length === 0) {
        errors.push(`${mPrefix}: missing or invalid "name"`);
      }

      validatedModels++;
    }

    validatedProviders++;
  }

  console.log(`Validated ${validatedProviders} providers, ${validatedModels} models`);
  return errors;
}

async function main() {
  try {
    const { raw, data } = await fetchModels();

    const errors = validate(data);
    if (errors.length > 0) {
      console.error("Validation failed with the following errors:");
      for (const err of errors) {
        console.error(`  - ${err}`);
      }
      process.exit(1);
    }

    console.log("Validation passed.");

    // Read existing file to compare
    try {
      const existingSize = Buffer.byteLength(readFileSync(TARGET_PATH, "utf-8"), "utf-8");
      const newSize = Buffer.byteLength(raw, "utf-8");
      console.log(`Existing file: ${(existingSize / 1024).toFixed(1)} KB, New data: ${(newSize / 1024).toFixed(1)} KB`);
    } catch {
      console.log("No existing models.dev.json found, creating new file.");
    }

    writeFileSync(TARGET_PATH, raw, "utf-8");
    console.log(`Successfully updated ${TARGET_PATH}`);
  } catch (err) {
    console.error(`Failed to update models: ${err.message}`);
    process.exit(1);
  }
}

main();
