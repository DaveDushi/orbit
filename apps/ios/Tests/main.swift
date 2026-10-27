import Foundation
import XCTest

final class ServerAddressTests: XCTestCase {
    private let server = URL(string: "https://orbit.example.com")!
    private let probe = URL(string: "https://orbit.example.com/api/v1/setup/status")!

    private func check(
        _ status: Int = 200, url: String? = nil,
        type: String? = "application/json", body: String = #"{"complete":true}"#
    ) -> ServerAddress.Verification {
        ServerAddress.verification(
            status: status, url: url.flatMap(URL.init(string:)) ?? probe,
            contentType: type, body: Data(body.utf8), requestedURL: probe
        )
    }

    func testDirectOrbit() {
        XCTAssertEqual(check(), .verified(server))
        XCTAssertEqual(check(body: #"{"complete":false}"#), .verified(server))
    }

    func testCanonicalRedirect() {
        XCTAssertEqual(check(url: "https://www.example.com/api/v1/setup/status"),
                       .verified(URL(string: "https://www.example.com")!))
    }

    func testExeGatewayRequiresLoginInsteadOfRejectingOrbit() {
        XCTAssertEqual(check(401, url: "https://exe.dev/auth?return_host=orbit.example.com",
                             type: "text/html; charset=utf-8", body: "<html>Sign in to continue</html>"),
                       .authenticationRequired)
        XCTAssertEqual(check(403), .authenticationRequired)
    }

    func testSuccessfulHTMLLoginPageIsNotAnOrbitServer() {
        XCTAssertEqual(check(url: "https://exe.dev/auth", type: "text/html", body: "<html>Sign in</html>"),
                       .authenticationRequired)
        XCTAssertEqual(check(type: "text/html", body: "<html>Sign in</html>"), .authenticationRequired)
    }

    func testProviderCannotBecomeTheSavedServer() {
        XCTAssertEqual(check(url: "https://exe.dev/auth"), .authenticationRequired)
        XCTAssertEqual(check(url: "https://orbit.example.com/login"), .authenticationRequired)
    }

    func testOnlySetupBooleanVerifiesOrbit() {
        for body in ["{}", #"{"complete":"true"}"#, #"{"complete":null}"#, "not JSON"] {
            XCTAssertEqual(check(body: body), .notOrbit)
        }
    }

    func testErrorPagesDoNotVerifyOrStartLoginLoops() {
        XCTAssertEqual(check(404, type: "text/html", body: "Not found"), .notOrbit)
        XCTAssertEqual(check(503, type: "text/html", body: "Unavailable"), .unavailable(503))
        XCTAssertEqual(check(500), .unavailable(500))
    }

    func testOriginNormalization() {
        XCTAssertEqual(ServerAddress.origin(from: "  ORBIT.example.com/path?token=secret#x  "), server)
        XCTAssertEqual(ServerAddress.origin(from: "https://user:password@orbit.example.com:443/path"), server)
        XCTAssertEqual(ServerAddress.origin(from: "http://localhost:8888/path"), URL(string: "http://localhost:8888"))
        XCTAssertEqual(ServerAddress.origin(from: "http://192.168.1.2:8080"), URL(string: "http://192.168.1.2:8080"))
    }

    func testInsecurePublicAndNonWebSchemesStayRejected() {
        for input in ["http://orbit.example.com", "file:///etc/passwd", "javascript://example.com", "https://"] {
            XCTAssertNil(ServerAddress.origin(from: input))
        }
    }

    func testEmailSignInLinkPreservesOneTimeTokenAndRedirect() {
        let link = "https://exe.dev/auth/verify/token%2Fvalue?token=one%2Btime&redirect=%2Fapi%2Fv1%2Fsetup%2Fstatus&return_host=orbit.example.com#finish"
        XCTAssertEqual(ServerAddress.signInLink(from: " \n" + link + "\n ")?.absoluteString, link)
        XCTAssertEqual(ServerAddress.signInLink(from: link)?.host, "exe.dev")
    }

    func testEmailSignInLinkRejectsUnsafeOrIncompleteInput() {
        for link in ["", "exe.dev/auth", "http://exe.dev/auth", "javascript:alert(1)",
                     "file:///tmp/login", "https://", "https://user:password@exe.dev/auth",
                     "https://exe.dev/auth\nhttps://other.example.com"] {
            XCTAssertNil(ServerAddress.signInLink(from: link))
        }
    }

    func testOriginComparisonKeepsSchemeAndPortBoundaries() {
        XCTAssertTrue(ServerAddress.isSameOrigin(URL(string: "https://ORBIT.example.com:443/path")!, as: server))
        XCTAssertFalse(ServerAddress.isSameOrigin(URL(string: "http://orbit.example.com")!, as: server))
        XCTAssertFalse(ServerAddress.isSameOrigin(URL(string: "https://orbit.example.com:8443")!, as: server))
        XCTAssertFalse(ServerAddress.isSameOrigin(URL(string: "https://exe.dev")!, as: server))
    }
}

XCTMain([testCase([
    ("testDirectOrbit", ServerAddressTests.testDirectOrbit),
    ("testCanonicalRedirect", ServerAddressTests.testCanonicalRedirect),
    ("testExeGatewayRequiresLoginInsteadOfRejectingOrbit", ServerAddressTests.testExeGatewayRequiresLoginInsteadOfRejectingOrbit),
    ("testSuccessfulHTMLLoginPageIsNotAnOrbitServer", ServerAddressTests.testSuccessfulHTMLLoginPageIsNotAnOrbitServer),
    ("testProviderCannotBecomeTheSavedServer", ServerAddressTests.testProviderCannotBecomeTheSavedServer),
    ("testOnlySetupBooleanVerifiesOrbit", ServerAddressTests.testOnlySetupBooleanVerifiesOrbit),
    ("testErrorPagesDoNotVerifyOrStartLoginLoops", ServerAddressTests.testErrorPagesDoNotVerifyOrStartLoginLoops),
    ("testOriginNormalization", ServerAddressTests.testOriginNormalization),
    ("testInsecurePublicAndNonWebSchemesStayRejected", ServerAddressTests.testInsecurePublicAndNonWebSchemesStayRejected),
    ("testOriginComparisonKeepsSchemeAndPortBoundaries", ServerAddressTests.testOriginComparisonKeepsSchemeAndPortBoundaries),
    ("testEmailSignInLinkPreservesOneTimeTokenAndRedirect", ServerAddressTests.testEmailSignInLinkPreservesOneTimeTokenAndRedirect),
    ("testEmailSignInLinkRejectsUnsafeOrIncompleteInput", ServerAddressTests.testEmailSignInLinkRejectsUnsafeOrIncompleteInput),
]), shareTests])
