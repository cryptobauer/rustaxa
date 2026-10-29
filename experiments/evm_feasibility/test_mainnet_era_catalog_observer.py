"""Exact source guards for archive-only ordinary storage observation."""
import hashlib
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import mainnet_era_reference as era


class ArchiveCatalogPatch(unittest.TestCase):
    def prepare(self,raw):
        root=Path(tempfile.mkdtemp(prefix='catalog-patch-',dir=os.environ['RUN_ARTIFACT_DIR']))
        target=root/era.mixed.TRACE_TARGET;target.parent.mkdir(parents=True)
        target.write_bytes(raw)
        return root,target

    def original(self):
        return era.mixed.TRACE_NEEDLE+era.CATALOG_NEEDLE

    def test_exact_patch_preserves_existing_raw_observer_and_refuses_reapplication(self):
        original=self.original()
        raw=original.replace(era.mixed.TRACE_NEEDLE,era.mixed.TRACE_INSERTION)
        root,target=self.prepare(raw)
        with mock.patch.object(era.mixed,'TRACE_TARGET_SHA256',hashlib.sha256(original).hexdigest()):
            era.apply_storage_catalog_patch(root)
            self.assertEqual(target.read_bytes(),raw.replace(era.CATALOG_NEEDLE,era.CATALOG_INSERTION))
            helper=target.parent/'mainnet_era_storage_catalog_observer.go'
            self.assertEqual(helper.read_bytes(),era.CATALOG_HELPER)
            with self.assertRaisesRegex(ValueError,'source identity'):
                era.apply_storage_catalog_patch(root)

    def test_unknown_engine_bytes_and_duplicate_needle_reject_before_mutation(self):
        for original,changed in [(self.original(),self.original()+b'// unknown engine delta\n'),
                                 (self.original()+era.CATALOG_NEEDLE,None)]:
            raw=(changed if changed is not None else original).replace(era.mixed.TRACE_NEEDLE,era.mixed.TRACE_INSERTION)
            root,target=self.prepare(raw)
            with mock.patch.object(era.mixed,'TRACE_TARGET_SHA256',hashlib.sha256(original).hexdigest()):
                with self.assertRaises(ValueError):era.apply_storage_catalog_patch(root)
            self.assertEqual(target.read_bytes(),raw)
            self.assertFalse((target.parent/'mainnet_era_storage_catalog_observer.go').exists())

    def test_existing_helper_is_never_overwritten(self):
        original=self.original();raw=original.replace(era.mixed.TRACE_NEEDLE,era.mixed.TRACE_INSERTION)
        root,target=self.prepare(raw);helper=target.parent/'mainnet_era_storage_catalog_observer.go'
        helper.write_bytes(b'preserved earlier evidence')
        with mock.patch.object(era.mixed,'TRACE_TARGET_SHA256',hashlib.sha256(original).hexdigest()):
            with self.assertRaises(FileExistsError):era.apply_storage_catalog_patch(root)
        self.assertEqual(target.read_bytes(),raw)
        self.assertEqual(helper.read_bytes(),b'preserved earlier evidence')


if __name__=='__main__':unittest.main()
