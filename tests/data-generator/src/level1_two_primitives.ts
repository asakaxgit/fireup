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

// Create Level 1 test data: documents with two primitive fields
console.log('\n📝 Creating Level 1 test data (two primitives per document)...');

// Document with two string fields
const stringStringDocRef = db.collection('level1_test').doc('string_string_doc');
await stringStringDocRef.set({
  field1: 'hello',
  field2: 'world'
});

// Document with two number fields
const numberNumberDocRef = db.collection('level1_test').doc('number_number_doc');
await numberNumberDocRef.set({
  field1: 42,
  field2: 100
});

// Document with string and number fields
const stringNumberDocRef = db.collection('level1_test').doc('string_number_doc');
await stringNumberDocRef.set({
  field1: 'test',
  field2: 123
});

// Document with number and string fields (different order)
const numberStringDocRef = db.collection('level1_test').doc('number_string_doc');
await numberStringDocRef.set({
  field1: 999,
  field2: 'value'
});

// Verify data was written
console.log('\n🔍 Verifying data was written...');
const snapshot = await db.collection('level1_test').get();
console.log(`Level 1 test collection has ${snapshot.size} documents:`);
snapshot.forEach(doc => {
  console.log(`  - ${doc.id}: ${JSON.stringify(doc.data())}`);
});

console.log('\n✅ Level 1 test data generation completed successfully!');
console.log('📊 Generated data:');
console.log('  - 1 document with two string fields: "hello", "world"');
console.log('  - 1 document with two number fields: 42, 100');
console.log('  - 1 document with string and number: "test", 123');
console.log('  - 1 document with number and string: 999, "value"');
