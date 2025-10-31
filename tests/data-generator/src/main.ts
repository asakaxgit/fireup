#!/usr/bin/env node

const FIRESTORE_HOST = process.env["FIRESTORE_EMULATOR_HOST"] ?? (() => { throw new Error("FIRESTORE_EMULATOR_HOST environment variable is required") })()

import { getFirestore } from 'firebase-admin/firestore';
import { initializeApp, getApps } from 'firebase-admin/app';

// You still need to initialize the app, but can do it minimally
if (getApps().length === 0) {
  initializeApp({
    projectId: 'demo-no-project' // match the emulator's detected project ID
  });
}

console.log(`Connecting to Firestore emulator at: ${FIRESTORE_HOST}`)

const db = getFirestore();
db.settings({
  host: FIRESTORE_HOST,
  ssl: false
});

// Test connection first
try {
  console.log('Testing connection to emulator...');
  const app = getApps()[0];
  console.log(`Using project ID: ${app?.options.projectId || 'unknown'}`);
} catch (error) {
  console.error('Failed to connect to emulator:', error);
  process.exit(1);
}

const docRef = db.collection('users').doc('alovelace');

await docRef.set({
  first: 'Ada',
  last: 'Lovelace',
  born: 1815
});

const aTuringRef = db.collection('users').doc('aturing');

await aTuringRef.set({
  'first': 'Alan',
  'middle': 'Mathison',
  'last': 'Turing',
  'born': 1912
});

const citiesRef = db.collection('cities');

await citiesRef.listDocuments().then(c => {
    console.log(c.length)
    c.forEach(doc=> {
    console.log(doc.id)
  })
});

await citiesRef.doc('SF').set({
  name: 'San Francisco', state: 'CA', country: 'USA',
  capital: false, population: 860000
});
await citiesRef.doc('LA').set({
  name: 'Los Angeles', state: 'CA', country: 'USA',
  capital: false, population: 3900000
});
await citiesRef.doc('DC').set({
  name: 'Washington, D.C.', state: null, country: 'USA',
  capital: true, population: 680000
});
await citiesRef.doc('TOK').set({
  name: 'Tokyo', state: null, country: 'Japan',
  capital: true, population: 9000000
});
await citiesRef.doc('BJ').set({
  name: 'Beijing', state: null, country: 'China',
  capital: true, population: 21500000
});


await citiesRef.listDocuments().then(c => {
    console.log(`Found ${c.length} cities after generation`)
    c.forEach(doc=> {
    console.log(`City: ${doc.id}`)
  })
});

// Verify data was written by reading it back
console.log("\n🔍 Verifying data was written...");
const usersSnapshot = await db.collection('users').get();
console.log(`Users collection has ${usersSnapshot.size} documents:`);
usersSnapshot.forEach(doc => {
  console.log(`  - ${doc.id}: ${JSON.stringify(doc.data())}`);
});

const citiesSnapshot = await db.collection('cities').get();
console.log(`Cities collection has ${citiesSnapshot.size} documents:`);
citiesSnapshot.forEach(doc => {
  console.log(`  - ${doc.id}: ${JSON.stringify(doc.data())}`);
});

console.log("\n✅ Data generation completed successfully!");
console.log("📊 Generated data:");
console.log("  - 2 users (alovelace, aturing)");
console.log("  - 5 cities (SF, LA, DC, TOK, BJ)");
console.log("🌐 You can view the data in the Firestore Emulator UI at:");
console.log("   http://localhost:14000/firestore/default/data");
console.log("   or http://127.0.0.1:14000/firestore/default/data");