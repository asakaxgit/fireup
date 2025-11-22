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

// Create Level 4 test data: arrays of primitive values
console.log('\n📝 Creating Level 4 test data (arrays of primitives)...');

const level4Collection = db.collection('level4_arrays');

// Document 1: Array of strings
const stringArrayDoc = level4Collection.doc('string_array_doc');
await stringArrayDoc.set({
  name: 'String Array Document',
  tags: ['javascript', 'typescript', 'rust', 'python'],
  categories: ['web', 'backend', 'systems']
});

// Document 2: Array of numbers
const numberArrayDoc = level4Collection.doc('number_array_doc');
await numberArrayDoc.set({
  name: 'Number Array Document',
  scores: [95, 87, 92, 88, 91],
  prices: [19.99, 29.99, 39.99],
  quantities: [10, 25, 50, 100]
});

// Document 3: Array of booleans
const booleanArrayDoc = level4Collection.doc('boolean_array_doc');
await booleanArrayDoc.set({
  name: 'Boolean Array Document',
  flags: [true, false, true, true, false],
  enabled: [true, true, false]
});

// Document 4: Mixed primitive arrays in one document
const mixedArrayDoc = level4Collection.doc('mixed_arrays_doc');
await mixedArrayDoc.set({
  name: 'Mixed Arrays Document',
  tags: ['admin', 'user', 'guest'],
  scores: [100, 95, 87],
  active: [true, false, true]
});

// Document 5: Empty arrays
const emptyArrayDoc = level4Collection.doc('empty_arrays_doc');
await emptyArrayDoc.set({
  name: 'Empty Arrays Document',
  tags: [],
  scores: [],
  flags: []
});

// Document 6: Single element arrays
const singleElementDoc = level4Collection.doc('single_element_doc');
await singleElementDoc.set({
  name: 'Single Element Arrays',
  tags: ['only'],
  scores: [42],
  flags: [true]
});

// Document 7: Large arrays
const largeArrayDoc = level4Collection.doc('large_array_doc');
await largeArrayDoc.set({
  name: 'Large Array Document',
  items: Array.from({ length: 20 }, (_, i) => `item_${i + 1}`),
  numbers: Array.from({ length: 15 }, (_, i) => i * 2)
});

// Verify data was written
console.log('\n🔍 Verifying data was written...');
const snapshot = await level4Collection.get();
console.log(`Level 4 arrays collection has ${snapshot.size} documents:`);
snapshot.forEach(doc => {
  const data = doc.data();
  console.log(`  - ${doc.id}: ${JSON.stringify(data, null, 2)}`);
});

console.log('\n✅ Level 4 arrays data generation completed successfully!');
console.log('📊 Generated data:');
console.log('  - 1 document with string arrays (tags, categories)');
console.log('  - 1 document with number arrays (scores, prices, quantities)');
console.log('  - 1 document with boolean arrays (flags, enabled)');
console.log('  - 1 document with mixed primitive arrays');
console.log('  - 1 document with empty arrays');
console.log('  - 1 document with single element arrays');
console.log('  - 1 document with large arrays');
console.log('🌐 You can view the data in the Firestore Emulator UI at:');
console.log('   http://localhost:14000/firestore/default/data');
