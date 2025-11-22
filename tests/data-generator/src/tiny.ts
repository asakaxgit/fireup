#!/usr/bin/env node

const FIRESTORE_HOST = process.env["FIRESTORE_EMULATOR_HOST"] ?? (() => { throw new Error("FIRESTORE_EMULATOR_HOST environment variable is required") })()

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

// Create minimal test data: one string and one number
console.log('\n📝 Creating minimal test data...');

// Document with just one string field
const stringDocRef = db.collection('tiny_test').doc('string_doc');
await stringDocRef.set({
  value: 'hello'
});

// Document with just one number field
const numberDocRef = db.collection('tiny_test').doc('number_doc');
await numberDocRef.set({
  value: 42
});

// Verify data was written
console.log('\n🔍 Verifying data was written...');
const snapshot = await db.collection('tiny_test').get();
console.log(`Tiny test collection has ${snapshot.size} documents:`);
snapshot.forEach(doc => {
  console.log(`  - ${doc.id}: ${JSON.stringify(doc.data())}`);
});

console.log('\n✅ Tiny test data generation completed successfully!');
console.log('📊 Generated data:');
console.log('  - 1 document with string value: "hello"');
console.log('  - 1 document with number value: 42');
