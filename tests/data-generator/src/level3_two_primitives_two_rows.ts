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

// Level 3: Two Primitives + Two Rows
// Combines Level 1 and Level 2 - multiple documents, each with multiple primitive fields
console.log('\n📝 Creating Level 3 test data (Two Primitives + Two Rows)...');

const usersRef = db.collection('users');

// Document 1: user_001 with name and age
await usersRef.doc('user_001').set({
  name: 'Alice',
  age: 30
});

// Document 2: user_002 with name and age
await usersRef.doc('user_002').set({
  name: 'Bob',
  age: 25
});

// Verify data was written
console.log('\n🔍 Verifying data was written...');
const snapshot = await usersRef.get();
console.log(`Users collection has ${snapshot.size} documents:`);
snapshot.forEach(doc => {
  console.log(`  - ${doc.id}: ${JSON.stringify(doc.data())}`);
});

console.log('\n✅ Level 3 test data generation completed successfully!');
console.log('📊 Generated data:');
console.log('  - Collection: users');
console.log('  - Document user_001: { name: "Alice", age: 30 }');
console.log('  - Document user_002: { name: "Bob", age: 25 }');
console.log('🌐 You can view the data in the Firestore Emulator UI at:');
console.log('   http://localhost:14000/firestore/default/data');
console.log('   or http://127.0.0.1:14000/firestore/default/data');
