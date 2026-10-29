import datetime
import importlib.util
import plistlib
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("device_signing", Path(__file__).resolve().parents[1] / "tools/prepare-device-signing.py")
signing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signing)


class SigningTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.app = self.root / "Orbit.app"
        self.extension = self.app / "PlugIns/OrbitShare.appex"
        self.extension.mkdir(parents=True)
        self.write(self.app / "Info.plist", {"CFBundleIdentifier": "dev.orbit.ios"})
        self.write(self.extension / "Info.plist", {"CFBundleIdentifier": "dev.orbit.ios.share"})
        self.stage = self.root / "stage"
        self.stage.mkdir()
        self.profiles = self.root / "profiles"
        self.profiles.mkdir()
        self.identities = self.root / "identities"
        self.identities.mkdir()
        self.write(self.identities / "ABCDEFGHIJ", {"certificate": b"fixture-cert", "privateKey": {"data": b"fixture-key"}})
        self.entitlements = self.root / "Sharing.entitlements"
        self.write(self.entitlements, {"keychain-access-groups": ["$(AppIdentifierPrefix)dev.orbit.ios.shared"]})
        self.make_profile("main", "XTL-ABCDEFGHIJ.dev.orbit.ios")
        self.make_profile("share", "XTL-ABCDEFGHIJ.dev.orbit.ios.share")

    def tearDown(self):
        self.temp.cleanup()

    @staticmethod
    def write(path, value):
        path.write_bytes(plistlib.dumps(value))

    def make_profile(self, name, identifier, allowed=None, device="fixture-device", expired=False):
        profile = {
            "ApplicationIdentifierPrefix": ["ABCDEFGHIJ"], "TeamIdentifier": ["ABCDEFGHIJ"],
            "ProvisionedDevices": [device], "DeveloperCertificates": [b"fixture-cert"],
            "ExpirationDate": datetime.datetime.now(datetime.UTC).replace(tzinfo=None)
                + datetime.timedelta(days=-1 if expired else 1),
            "Entitlements": {"application-identifier": "ABCDEFGHIJ." + identifier,
                             "keychain-access-groups": allowed if allowed is not None else ["ABCDEFGHIJ.*"]},
        }
        self.write(self.profiles / (name + ".mobileprovision"), profile)

    def prepare(self):
        return signing.prepare(self.app, self.stage, self.profiles, self.identities, self.entitlements, "fixture-device")

    def test_both_targets_keep_shared_group_and_existing_signed_ids(self):
        self.assertEqual(self.prepare(), 0)
        for name in ["Orbit", "OrbitShare"]:
            entitlements = plistlib.loads((self.stage / (name + ".entitlements")).read_bytes())
            self.assertEqual(entitlements["keychain-access-groups"], ["ABCDEFGHIJ.dev.orbit.ios.shared"])
        app = self.stage / "Payload/Orbit.app"
        self.assertEqual(plistlib.loads((app / "Info.plist").read_bytes())["CFBundleIdentifier"], "XTL-ABCDEFGHIJ.dev.orbit.ios")
        self.assertEqual(plistlib.loads((app / "PlugIns/OrbitShare.appex/Info.plist").read_bytes())["CFBundleIdentifier"], "XTL-ABCDEFGHIJ.dev.orbit.ios.share")
        self.assertEqual((self.stage / "key.pem").stat().st_mode & 0o777, 0o600)
        self.assertEqual(plistlib.loads((self.app / "Info.plist").read_bytes())["CFBundleIdentifier"], "dev.orbit.ios")

    def test_missing_extension_profile_requests_provisioning_without_exporting_key(self):
        (self.profiles / "share.mobileprovision").unlink()
        self.assertEqual(self.prepare(), 75)
        self.assertFalse((self.stage / "key.pem").exists())

    def test_expired_or_wrong_device_profile_is_not_used(self):
        self.make_profile("share", "XTL-ABCDEFGHIJ.dev.orbit.ios.share", expired=True)
        self.assertEqual(self.prepare(), 75)
        self.make_profile("share", "XTL-ABCDEFGHIJ.dev.orbit.ios.share", device="other-device")
        self.assertEqual(self.prepare(), 75)

    def test_certificate_must_match_both_profiles(self):
        self.write(self.identities / "ABCDEFGHIJ", {"certificate": b"different-cert", "privateKey": {"data": b"fixture-key"}})
        self.assertEqual(self.prepare(), 75)

    def test_shared_group_must_be_permitted_by_each_profile(self):
        self.make_profile("share", "XTL-ABCDEFGHIJ.dev.orbit.ios.share", allowed=["ABCDEFGHIJ.unrelated"])
        with self.assertRaises(ValueError):
            self.prepare()
        self.assertFalse((self.stage / "key.pem").exists())


if __name__ == "__main__":
    unittest.main()
