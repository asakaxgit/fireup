#!/usr/bin/env node

const FIRESTORE_HOST = process.env["FIRESTORE_EMULATOR_HOST"] ?? (() => { throw new Error("FIRESTORE_EMULATOR_HOST environment variable is required") })();

import { getFirestore } from 'firebase-admin/firestore';
import { initializeApp, getApps } from 'firebase-admin/app';

// Initialize the app
if (getApps().length === 0) {
  initializeApp({
    projectId: 'demo-no-project'
  });
}

console.log(`Connecting to Firestore emulator at: ${FIRESTORE_HOST}`)

const db = getFirestore();
db.settings({
  host: FIRESTORE_HOST,
  ssl: false
});

// Test connection
try {
  console.log('Testing connection to emulator...');
  const app = getApps()[0];
  console.log(`Using project ID: ${app?.options.projectId || 'unknown'}`);
} catch (error) {
  console.error('Failed to connect to emulator:', error);
  process.exit(1);
}

// Create Level 2 test data: two documents with the same single field structure
console.log('\n📝 Creating Level 2 test data (Two Rows - Single Field, Multiple Documents)...');

// First document with a string field
const item1Ref = db.collection('simple_items').doc('item_001');
await item1Ref.set({
  name: 'Apple'
});

// Second document with the same field structure
const item2Ref = db.collection('simple_items').doc('item_002');
await item2Ref.set({
  name: 'Banana'
});

// Verify data was written
console.log('\n🔍 Verifying data was written...');
const snapshot = await db.collection('simple_items').get();
console.log(`Simple items collection has ${snapshot.size} documents:`);
snapshot.forEach(doc => {
  console.log(`  - ${doc.id}: ${JSON.stringify(doc.data())}`);
});

console.log('\n✅ Level 2 test data generation completed successfully!');
console.log('📊 Generated data:');
console.log('  - Collection: simple_items');
console.log('  - 2 documents with the same single field structure');
console.log('  - Document item_001: { name: "Apple" }');
console.log('  - Document item_002: { name: "Banana" }');
