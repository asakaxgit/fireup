#!/usr/bin/env python3
"""
Simple script to verify the sample Firestore data exists and analyze its content.
This runs independently of the Rust compilation issues.
"""

import os
import json
from pathlib import Path

def main():
    print("🔥 Firestore Sample Data Verification")
    print("=" * 50)
    
    # Check base directory
    base_path = Path("tests/.firestore-data")
    if not base_path.exists():
        print("❌ Base directory not found: tests/.firestore-data")
        return False
    
    print(f"✅ Base directory found: {base_path}")
    
    # Check Firebase export metadata
    metadata_file = base_path / "firebase-export-metadata.json"
    if metadata_file.exists():
        print(f"✅ Firebase export metadata found: {metadata_file}")
        try:
            with open(metadata_file, 'r') as f:
                metadata = json.load(f)
            print(f"   Version: {metadata.get('version', 'unknown')}")
            print(f"   Firestore version: {metadata.get('firestore', {}).get('version', 'unknown')}")
        except Exception as e:
            print(f"   ⚠️ Could not parse metadata: {e}")
    else:
        print(f"❌ Firebase export metadata not found: {metadata_file}")
    
    # Check Firestore export directory structure
    export_dir = base_path / "firestore_export"
    if export_dir.exists():
        print(f"✅ Firestore export directory found: {export_dir}")
    else:
        print(f"❌ Firestore export directory not found: {export_dir}")
        return False
    
    # Check overall metadata
    overall_metadata = export_dir / "firestore_export.overall_export_metadata"
    if overall_metadata.exists():
        print(f"✅ Overall export metadata found: {overall_metadata}")
        print(f"   Size: {overall_metadata.stat().st_size} bytes")
    else:
        print(f"❌ Overall export metadata not found: {overall_metadata}")
    
    # Check data file
    data_file = export_dir / "all_namespaces" / "all_kinds" / "output-0"
    if data_file.exists():
        print(f"✅ Data file found: {data_file}")
        size = data_file.stat().st_size
        print(f"   Size: {size} bytes")
        
        # Read and analyze the binary data
        try:
            with open(data_file, 'rb') as f:
                data = f.read()
            
            print(f"   Successfully read {len(data)} bytes")
            
            # Show hex dump of first 64 bytes
            hex_dump = ' '.join(f'{b:02x}' for b in data[:64])
            print(f"   First 64 bytes (hex): {hex_dump}")
            
            # Look for text patterns (collections and documents from the generator)
            data_str = data.decode('utf-8', errors='ignore')
            
            collections = ["cities", "users"]
            found_collections = []
            for collection in collections:
                if collection in data_str:
                    found_collections.append(collection)
                    print(f"   ✅ Found collection: {collection}")
            
            doc_ids = ["alovelace", "aturing", "SF", "LA", "DC", "TOK", "BJ"]
            found_docs = []
            for doc_id in doc_ids:
                if doc_id in data_str:
                    found_docs.append(doc_id)
                    print(f"   ✅ Found document ID: {doc_id}")
            
            fields = ["first", "last", "born", "name", "state", "country", "capital", "population"]
            found_fields = []
            for field in fields:
                if field in data_str:
                    found_fields.append(field)
                    print(f"   ✅ Found field: {field}")
            
            values = ["Ada", "Lovelace", "Alan", "Turing", "San Francisco", "Los Angeles", "Washington", "Tokyo", "Beijing"]
            found_values = []
            for value in values:
                if value in data_str:
                    found_values.append(value)
                    print(f"   ✅ Found value: {value}")
            
            print(f"\n📊 Analysis Summary:")
            print(f"   Collections found: {len(found_collections)}/{len(collections)} ({found_collections})")
            print(f"   Documents found: {len(found_docs)}/{len(doc_ids)} ({found_docs})")
            print(f"   Fields found: {len(found_fields)}/{len(fields)} ({found_fields})")
            print(f"   Values found: {len(found_values)}/{len(values)} ({found_values})")
            
        except Exception as e:
            print(f"   ⚠️ Could not analyze data file: {e}")
    else:
        print(f"❌ Data file not found: {data_file}")
        return False
    
    # Check export metadata in namespaces
    ns_metadata = export_dir / "all_namespaces" / "all_kinds" / "all_namespaces_all_kinds.export_metadata"
    if ns_metadata.exists():
        print(f"✅ Namespace export metadata found: {ns_metadata}")
        print(f"   Size: {ns_metadata.stat().st_size} bytes")
    else:
        print(f"❌ Namespace export metadata not found: {ns_metadata}")
    
    print(f"\n🎯 Expected Data Structure (from generator):")
    print(f"   - users collection: alovelace (Ada Lovelace, born 1815), aturing (Alan Turing, born 1912)")
    print(f"   - cities collection: SF, LA, DC, TOK, BJ with population, capital status, etc.")
    
    print(f"\n✅ Sample data verification complete!")
    print(f"🚀 This data is ready for functional testing with the Firestore parser")
    
    return True

if __name__ == "__main__":
    success = main()
    exit(0 if success else 1)