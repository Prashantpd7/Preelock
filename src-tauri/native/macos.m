#import <AppKit/AppKit.h>
#import <AVFoundation/AVFoundation.h>
#import <Vision/Vision.h>
#import <CoreGraphics/CoreGraphics.h>
#import <ApplicationServices/ApplicationServices.h>
#import <mach/mach_time.h>
#import <objc/runtime.h>
#import <Carbon/Carbon.h>

int preelock_accessibility_status(void);

static void onMain(dispatch_block_t work) {
    if ([NSThread isMainThread]) {
        work();
    } else if (NSApp != nil && [NSApp isRunning]) {
        dispatch_sync(dispatch_get_main_queue(), work);
    } else {
        work();
    }
}

// MARK: - Application lifecycle identity
//
// One bundle, several process modes (GUI, --background, --tray,
// --attention). LaunchServices identifies an application by bundle id, so a
// headless mode that touches NSApplication registers a SECOND instance of
// com.preelock.desktop next to the GUI (``lsappinfo`` reports it as
// BackgroundOnly). macOS then resolves "Preelock" to that daemon: activation
// and Apple Events are delivered to a process with no window and no run loop,
// which is how `open -a Preelock.app` used to turn the background engine into a
// frontmost, windowless, unquittable app. Headless modes therefore never
// initialize NSApplication (see `preelock_set_activation_policy_prohibited`,
// removed). This helper block is the single source of truth for that identity.
static NSString *vyBundleIdentifier(void) {
    return NSBundle.mainBundle.bundleIdentifier ?: @"";
}
static BOOL vyIsBundledApp(void) {
    return vyBundleIdentifier().length > 0 &&
        [NSBundle.mainBundle.bundleURL.pathExtension.lowercaseString isEqualToString:@"app"];
}
static NSString *vyDefaultBundleIdentifier(void) {
    return @"com.preelock.desktop";
}
static NSString *vyDataDirectory(void) {
    NSString *override = NSProcessInfo.processInfo.environment[@"PREELOCK_DATA_DIR"];
    if (override.length > 0) return override;
    NSString *ident = vyBundleIdentifier();
    if (ident.length == 0) ident = vyDefaultBundleIdentifier();
    return [NSString stringWithFormat:@"%@/Library/Application Support/%@", NSHomeDirectory(), ident];
}
// Lifecycle diagnostics: one prefixed line per event on stderr, which launchd
// captures for the background service
// (~/Library/Logs/com.preelock.background.stderr.log) and which the Rust
// logger also mirrors into ~/Library/Logs/Preelock/lifecycle.log. Never log
// notification titles/bodies or other user content here.
static void vyLifecycleLog(NSString *format, ...) {
    va_list args;
    va_start(args, format);
    NSString *message = [[NSString alloc] initWithFormat:format arguments:args];
    va_end(args);
    NSLog(@"[preelock-lifecycle] %@", message);
}
static char *json(id object) {
    NSData *data = [NSJSONSerialization dataWithJSONObject:object options:0 error:nil];
    return strdup([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding].UTF8String ?: "null");
}
static NSString *displayNameForApp(NSRunningApplication *app) {
    // Prefer CFBundleDisplayName (e.g. "Visual Studio Code") over the raw
    // bundle/localized name (e.g. "Code").
    if (app.bundleIdentifier) {
        NSBundle *bundle = [NSBundle bundleWithIdentifier:app.bundleIdentifier];
        NSString *display = [bundle objectForInfoDictionaryKey:@"CFBundleDisplayName"];
        if ([display isKindOfClass:[NSString class]] && display.length > 0) return display;
    }
    return app.localizedName ?: app.bundleIdentifier;
}
static NSString *executablePathForApp(NSRunningApplication *app) {
    return app.executableURL.path ?: @"";
}
// Step 4: title of the frontmost app's window, via
// CGWindowListCopyWindowInfo. The on-screen list is front-to-back, so
// the first non-empty title owned by this pid is the front window's.
// Returns @"" when the app has no titled window (e.g. a menu-bar-only
// moment). The string is copied before releasing the window list.
static NSString *frontmostWindowTitle(pid_t pid) {
    CFArrayRef list = CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly, kCGNullWindowID);
    if (!list) return @"";
    NSString *title = @"";
    CFIndex count = CFArrayGetCount(list);
    for (CFIndex i = 0; i < count; i++) {
        CFDictionaryRef win = (CFDictionaryRef)CFArrayGetValueAtIndex(list, i);
        CFNumberRef owner = (CFNumberRef)CFDictionaryGetValue(win, kCGWindowOwnerPID);
        int64_t ownerPid = 0;
        if (!owner || !CFNumberGetValue(owner, kCFNumberSInt64Type, &ownerPid) || ownerPid != pid) continue;
        CFStringRef name = (CFStringRef)CFDictionaryGetValue(win, kCGWindowName);
        if (name && CFStringGetLength(name) > 0) { title = [(__bridge NSString *)name copy]; break; }
    }
    CFRelease(list);
    return title;
}
char *preelock_apps(void) {
    @autoreleasepool {
        __block char *result;
        onMain(^{
            NSMutableArray *apps = [NSMutableArray array];
            for (NSRunningApplication *app in NSWorkspace.sharedWorkspace.runningApplications) {
                if (app.activationPolicy != NSApplicationActivationPolicyRegular) continue;
                // Step 4: processes without a bundle id (raw executables,
                // dev binaries) are identified by their executable path,
                // so they can be picked and matched by path/folder rules.
                NSString *exe = executablePathForApp(app);
                NSString *ident = app.bundleIdentifier.length ? app.bundleIdentifier : exe;
                if (ident.length == 0) continue;
                NSString *name = displayNameForApp(app) ?: [exe lastPathComponent];
                [apps addObject:@{@"id":ident, @"name":name ?: @""}];
            }
            result = json(apps);
        });
        return result;
    }
}
char *preelock_sample(void) {
    @autoreleasepool {
        __block char *result;
        onMain(^{
            NSRunningApplication *app = NSWorkspace.sharedWorkspace.frontmostApplication;
            NSString *exe = executablePathForApp(app);
            NSString *bundlePath = app.bundleURL.path ?: @"";
            // Step 4: path is the bundle path for .app processes, the
            // executable path otherwise; id falls back to the
            // executable path for bundle-less processes (step 4.4), so
            // path/folder rules and explicit path entries can match
            // them. Title is the frontmost window's title (step 4.2).
            NSString *ident = app.bundleIdentifier.length ? app.bundleIdentifier : exe;
            NSString *path = bundlePath.length ? bundlePath : exe;
            boolean_t asleep = CGDisplayIsAsleep(CGMainDisplayID());
            NSDictionary *sessionDict = CFBridgingRelease(CGSessionCopyCurrentDictionary());
            BOOL locked = [sessionDict[@"CGSSessionScreenIsLocked"] boolValue];
            result = json(@{
                @"id": ident ?: @"",
                @"name":displayNameForApp(app) ?: @"",
                @"pid":@(app.processIdentifier),
                @"idle_ms":@((uint64_t)(CGEventSourceSecondsSinceLastEventType(kCGEventSourceStateCombinedSessionState, kCGAnyInputEventType)*1000)),
                @"path":path ?: @"",
                @"title":frontmostWindowTitle(app.processIdentifier),
                @"display_asleep": (asleep != 0) ? (id)kCFBooleanTrue : (id)kCFBooleanFalse,
                @"screen_locked": (locked != 0 || [ident isEqualToString:@"com.apple.loginwindow"]) ? (id)kCFBooleanTrue : (id)kCFBooleanFalse
            });
        });
        return result;
    }
}
int preelock_hide(int pid) {
    __block BOOL result = NO;
    onMain(^{ NSRunningApplication *app = [NSRunningApplication runningApplicationWithProcessIdentifier:pid]; if (app.activationPolicy == NSApplicationActivationPolicyRegular) result = [app hide]; });
    return result;
}
// ---- Native website identification (no browser extension) ----
// Reads only the front window's page URL: the first AXWebArea's AXURL
// (Chromium browsers and Safari expose the main frame there), or the
// address bar's value when the page has not exposed a web area yet.
// Page contents, window titles, form values and passwords are never read
// or returned; the Rust side validates what it gets and keeps only the
// hostname.
static BOOL axContainerRole(NSString *role) {
    static NSSet<NSString *> *roles;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        roles = [NSSet setWithArray:@[@"AXWindow", @"AXGroup", @"AXSplitGroup", @"AXScrollArea",
                                      @"AXTabGroup", @"AXLayoutArea", @"AXLayoutItem", @"AXToolbar",
                                      @"AXWebArea", @"AXUnknown"]];
    });
    return [roles containsObject:role];
}
static NSString *axStringAttr(AXUIElementRef element, CFStringRef attribute) {
    CFTypeRef value = NULL;
    if (AXUIElementCopyAttributeValue(element, attribute, &value) != kAXErrorSuccess || !value) return @"";
    NSString *result = @"";
    if (CFGetTypeID(value) == CFStringGetTypeID()) {
        result = [(__bridge NSString *)value copy];
    } else if (CFGetTypeID(value) == CFURLGetTypeID()) {
        result = [(__bridge NSURL *)value absoluteString] ?: @"";
    }
    CFRelease(value);
    return result;
}
// Depth- and node-bounded: a pathological accessibility tree can never
// stall the sampler, and containers only are descended into.
static NSString *axURLFromWebArea(AXUIElementRef element, int depth, int *budget) {
    if (depth > 24 || *budget <= 0) return @"";
    (*budget)--;
    NSString *role = axStringAttr(element, kAXRoleAttribute);
    if ([role isEqualToString:@"AXWebArea"]) {
        NSString *url = axStringAttr(element, kAXURLAttribute);
        if (url.length) return url;
    }
    if (depth > 0 && !axContainerRole(role)) return @"";
    CFTypeRef raw = NULL;
    if (AXUIElementCopyAttributeValue(element, kAXChildrenAttribute, &raw) != kAXErrorSuccess || !raw) return @"";
    NSString *found = @"";
    if (CFGetTypeID(raw) == CFArrayGetTypeID()) {
        CFArrayRef children = (CFArrayRef)raw;
        for (CFIndex i = 0; i < CFArrayGetCount(children) && *budget > 0; i++) {
            AXUIElementRef child = (AXUIElementRef)CFArrayGetValueAtIndex(children, i);
            if (!child || CFGetTypeID(child) != AXUIElementGetTypeID()) continue;
            found = axURLFromWebArea(child, depth + 1, budget);
            if (found.length) break;
        }
    }
    CFRelease(raw);
    return found;
}
// True only for text that looks like a web address, so a dotted version
// number (3.14) or a search query typed into the bar is never offered.
static BOOL looksLikeAddress(NSString *value) {
    if (value.length == 0 || value.length > 2048) return NO;
    if ([value containsString:@"://"]) return YES;
    NSRange cut = [value rangeOfCharacterFromSet:[NSCharacterSet characterSetWithCharactersInString:@"/?#"]];
    NSString *host = cut.location == NSNotFound ? value : [value substringToIndex:cut.location];
    NSRange colon = [host rangeOfString:@":"];
    if (colon.location != NSNotFound) host = [host substringToIndex:colon.location];
    NSArray<NSString *> *labels = [host componentsSeparatedByString:@"."];
    if (labels.count < 2) return NO;
    NSString *tld = labels.lastObject;
    if (tld.length < 2 || host.length <= tld.length + 1) return NO;
    for (NSUInteger i = 0; i < tld.length; i++) {
        unichar c = [tld characterAtIndex:i];
        if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z'))) return NO;
    }
    return YES;
}
static NSString *safariCurrentURL(void) {
    NSDictionary *error = nil;
    NSAppleScript *script = [[NSAppleScript alloc] initWithSource:
        @"tell application \"Safari\"\n"
        @"  if (count of windows) > 0 then\n"
        @"    return URL of current tab of front window\n"
        @"  end if\n"
        @"end tell"];
    NSAppleEventDescriptor *desc = [script executeAndReturnError:&error];
    if (desc && desc.stringValue) {
        return desc.stringValue;
    }
    return @"";
}
void preelock_redirect_safari_blocked(void) {
    @autoreleasepool {
        NSDictionary *error = nil;
        NSAppleScript *script = [[NSAppleScript alloc] initWithSource:
            @"tell application \"Safari\"\n"
            @"  if (count of windows) > 0 then\n"
            @"    try\n"
            @"      close current tab of front window\n"
            @"    on error\n"
            @"      set URL of current tab of front window to \"about:blank\"\n"
            @"    end try\n"
            @"  end if\n"
            @"end tell"];
        [script executeAndReturnError:&error];
    }
}
void preelock_redirect_browser_blocked(const char *appName, int pid) {
    @autoreleasepool {
        if (!appName || strlen(appName) == 0) return;
        NSString *name = [NSString stringWithUTF8String:appName];
        if ([name containsString:@"Safari"]) {
            preelock_redirect_safari_blocked();
            return;
        }
        // Maps bundle id or binary names to application name for AppleScript
        NSString *targetName = name;
        if ([name containsString:@"chrome"] || [name containsString:@"Chrome"]) {
            targetName = @"Google Chrome";
        } else if ([name containsString:@"brave"] || [name containsString:@"Brave"]) {
            targetName = @"Brave Browser";
        } else if ([name containsString:@"edge"] || [name containsString:@"Edge"]) {
            targetName = @"Microsoft Edge";
        } else if ([name containsString:@"arc"] || [name containsString:@"Arc"]) {
            targetName = @"Arc";
        } else if ([name containsString:@"comet"] || [name containsString:@"Comet"]) {
            targetName = @"Comet";
        }

        BOOL redirected = NO;
        NSString *source1 = [NSString stringWithFormat:
            @"tell application \"%@\"\n"
            @"  if (count of windows) > 0 then\n"
            @"    try\n"
            @"      close active tab of front window\n"
            @"    on error\n"
            @"      set URL of active tab of front window to \"about:blank\"\n"
            @"    end try\n"
            @"  end if\n"
            @"end tell", targetName];
        NSDictionary *error1 = nil;
        NSAppleScript *script1 = [[NSAppleScript alloc] initWithSource:source1];
        if ([script1 executeAndReturnError:&error1] != nil && error1 == nil) {
            redirected = YES;
        }

        if (!redirected) {
            NSString *source2 = [NSString stringWithFormat:
                @"tell application \"%@\"\n"
                @"  if (count of windows) > 0 then\n"
                @"    try\n"
                @"      close active tab of window 1\n"
                @"    on error\n"
                @"      set URL of active tab of window 1 to \"about:blank\"\n"
                @"    end try\n"
                @"  end if\n"
                @"end tell", targetName];
            NSDictionary *error2 = nil;
            NSAppleScript *script2 = [[NSAppleScript alloc] initWithSource:source2];
            if ([script2 executeAndReturnError:&error2] != nil && error2 == nil) {
                redirected = YES;
            }
        }

        // Reliable native fallback: send Cmd+W (close tab) keystroke directly to frontmost browser window
        pid_t targetPid = (pid_t)pid;
        if (targetPid <= 0) {
            NSRunningApplication *frontApp = NSWorkspace.sharedWorkspace.frontmostApplication;
            if (frontApp != nil) {
                targetPid = frontApp.processIdentifier;
            }
        }
        if (targetPid > 0) {
            CGEventSourceRef src = CGEventSourceCreate(kCGEventSourceStateHIDSystemState);
            if (src) {
                CGEventRef keyDown = CGEventCreateKeyboardEvent(src, (CGKeyCode)13, true); // key code 13 is 'w'
                CGEventSetFlags(keyDown, kCGEventFlagMaskCommand);
                CGEventRef keyUp = CGEventCreateKeyboardEvent(src, (CGKeyCode)13, false);
                CGEventSetFlags(keyUp, kCGEventFlagMaskCommand);
                CGEventPostToPid(targetPid, keyDown);
                CGEventPostToPid(targetPid, keyUp);
                CGEventPost(kCGHIDEventTap, keyDown);
                CGEventPost(kCGHIDEventTap, keyUp);
                CFRelease(keyDown);
                CFRelease(keyUp);
                CFRelease(src);
            }
        }
    }
}
static NSString *axAddressBar(AXUIElementRef element, int depth, int *budget) {
    if (depth > 12 || *budget <= 0) return @"";
    (*budget)--;
    NSString *role = axStringAttr(element, kAXRoleAttribute);
    if ([role isEqualToString:@"AXTextField"] || [role isEqualToString:@"AXComboBox"]) {
        NSString *value = axStringAttr(element, kAXValueAttribute);
        if (looksLikeAddress(value)) return value;
    }
    CFTypeRef raw = NULL;
    if (AXUIElementCopyAttributeValue(element, kAXChildrenAttribute, &raw) != kAXErrorSuccess || !raw) return @"";
    NSString *found = @"";
    if (CFGetTypeID(raw) == CFArrayGetTypeID()) {
        CFArrayRef children = (CFArrayRef)raw;
        for (CFIndex i = 0; i < CFArrayGetCount(children) && *budget > 0; i++) {
            AXUIElementRef child = (AXUIElementRef)CFArrayGetValueAtIndex(children, i);
            if (!child || CFGetTypeID(child) != AXUIElementGetTypeID()) continue;
            found = axAddressBar(child, depth + 1, budget);
            if (found.length) break;
        }
    }
    CFRelease(raw);
    return found;
}
char *preelock_web_sample(void) {
    @autoreleasepool {
        __block pid_t pid = 0;
        __block NSString *bundleId = @"";
        onMain(^{
            NSRunningApplication *app = NSWorkspace.sharedWorkspace.frontmostApplication;
            if (app) {
                pid = app.processIdentifier;
                bundleId = app.bundleIdentifier ?: @"";
            }
        });
        if (pid <= 0) return strdup("{\"url\":\"\",\"status\":\"no_url\"}");

        // If frontmost application is Safari, query directly via AppleScript.
        if ([bundleId isEqualToString:@"com.apple.Safari"]) {
            NSString *sUrl = safariCurrentURL();
            if (sUrl.length > 0) {
                return json(@{@"url": sUrl, @"status": @"ok"});
            }
        }

        if (preelock_accessibility_status() != 1) {
            return strdup("{\"url\":\"\",\"status\":\"permission_required\"}");
        }

        // The Accessibility query runs off the main thread: it can block
        // while the browser builds its tree, and Preelock's UI must not.
        AXUIElementRef application = AXUIElementCreateApplication(pid);
        if (!application) return strdup("{\"url\":\"\",\"status\":\"no_url\"}");
        NSString *url = @"";
        CFTypeRef window = NULL;
        if (AXUIElementCopyAttributeValue(application, kAXFocusedWindowAttribute, &window) == kAXErrorSuccess && window) {
            int budget = 600;
            url = axURLFromWebArea((AXUIElementRef)window, 0, &budget);
            if (url.length == 0) {
                int barBudget = 400;
                url = axAddressBar((AXUIElementRef)window, 0, &barBudget);
            }
            CFRelease(window);
        }
        CFRelease(application);
        return json(@{@"url": url ?: @"", @"status": @"ok"});
    }
}
// Authoritative, un-cached query of macOS Accessibility trust state from WindowServer / TCC daemon.
// Passing kAXTrustedCheckOptionPrompt: @NO bypasses HIServices stale process-local caching without showing any prompt.
int preelock_accessibility_status(void) {
    NSDictionary *options = @{(__bridge NSString *)kAXTrustedCheckOptionPrompt: @NO};
    Boolean trusted = AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)options);
    return trusted ? 1 : 0;
}
int preelock_web_permission(void) { return preelock_accessibility_status(); }
void preelock_web_request_permission(void) {
    onMain(^{
        if (preelock_accessibility_status() == 1) return;
        NSDictionary *options = @{(__bridge NSString *)kAXTrustedCheckOptionPrompt: @YES};
        AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)options);
    });
}
int preelock_camera_permission(void) { return (int)[AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo]; }
void preelock_request_camera(void) { [AVCaptureDevice requestAccessForMediaType:AVMediaTypeVideo completionHandler:^(BOOL granted) { (void)granted; }]; }

@interface VYCamera : NSObject <AVCaptureVideoDataOutputSampleBufferDelegate>
@property AVCaptureSession *session;
@property dispatch_queue_t queue;
@property NSString *status;
@property double lastFrame;
@property double lastSeen;
@end
@implementation VYCamera
- (instancetype)init { if ((self=[super init])) { _queue=dispatch_queue_create("com.preelock.camera", DISPATCH_QUEUE_SERIAL); _status=@"off"; } return self; }
- (void)captureOutput:(AVCaptureOutput *)output didOutputSampleBuffer:(CMSampleBufferRef)buffer fromConnection:(AVCaptureConnection *)connection {
    (void)output; (void)connection;
    double now = NSProcessInfo.processInfo.systemUptime;
    if (now-self.lastFrame < 1) return;
    self.lastFrame=now;
    CVPixelBufferRef frame=CMSampleBufferGetImageBuffer(buffer);
    if (!frame) return;
    VNDetectHumanRectanglesRequest *request=[VNDetectHumanRectanglesRequest new]; request.upperBodyOnly=YES;
    VNImageRequestHandler *handler=[[VNImageRequestHandler alloc] initWithCVPixelBuffer:frame options:@{}];
    NSError *error=nil;
    BOOL ok=[handler performRequests:@[request] error:&error];
    if (!ok) { self.status=@"analysis_error"; return; }
    BOOL present=NO;
    for (VNHumanObservation *person in request.results) { if (person.confidence >= 0.6) present=YES; }
    if (present) self.lastSeen=now;
    self.status=present ? @"person_detected" : (now-self.lastSeen >= 30 ? @"extended_absence" : @"person_not_detected");
}
@end
static VYCamera *camera;
static VYCamera *getCamera(void) { static dispatch_once_t once; dispatch_once(&once, ^{ camera=[VYCamera new]; }); return camera; }
int preelock_camera_start(void) {
    if (preelock_camera_permission()!=AVAuthorizationStatusAuthorized) return 0;
    VYCamera *c=getCamera(); __block BOOL ok=NO;
    dispatch_sync(c.queue, ^{
        if (c.session.isRunning) { ok=YES; return; }
        AVCaptureDevice *device=[AVCaptureDevice defaultDeviceWithMediaType:AVMediaTypeVideo];
        if (!device) { c.status=@"no_camera"; return; }
        NSError *error=nil; AVCaptureDeviceInput *input=[AVCaptureDeviceInput deviceInputWithDevice:device error:&error];
        AVCaptureSession *session=[AVCaptureSession new]; session.sessionPreset=AVCaptureSessionPreset640x480;
        AVCaptureVideoDataOutput *output=[AVCaptureVideoDataOutput new]; output.alwaysDiscardsLateVideoFrames=YES;
        if (!input || ![session canAddInput:input] || ![session canAddOutput:output]) { c.status=@"camera_unavailable"; return; }
        [session addInput:input]; [session addOutput:output]; [output setSampleBufferDelegate:c queue:c.queue];
        c.lastSeen=NSProcessInfo.processInfo.systemUptime; c.lastFrame=0; c.status=@"starting"; c.session=session;
        [session startRunning]; ok=session.isRunning;
        if (!ok) c.status=@"camera_unavailable";
    }); return ok;
}
void preelock_camera_stop(void) {
    VYCamera *c=getCamera(); dispatch_sync(c.queue, ^{ [c.session stopRunning]; c.session=nil; c.status=@"off"; });
}
char *preelock_camera_status(void) {
    VYCamera *c=getCamera(); __block char *result;
    dispatch_sync(c.queue, ^{
        NSString *status=c.status;
        if (c.session && !c.session.isRunning) status=@"camera_interrupted";
        else if (c.session && NSProcessInfo.processInfo.systemUptime-(c.lastFrame>0 ? c.lastFrame : c.lastSeen)>10) status=@"camera_stalled";
        result=strdup(status.UTF8String);
    }); return result;
}
void preelock_free(char *data) { free(data); }

// Settings integrations. All panels and AppKit work stay on the main thread.
#import <UserNotifications/UserNotifications.h>
#import <ServiceManagement/ServiceManagement.h>
char *preelock_settings_info(void) {
    @autoreleasepool {
        // UNUserNotificationCenter crashes the process when there is no .app
        // bundle (dev mode) — check first and report unavailable instead.
        BOOL bundled = NSBundle.mainBundle.bundleIdentifier != nil &&
            [NSBundle.mainBundle.bundleURL.pathExtension.lowercaseString isEqualToString:@"app"];
        __block NSInteger auth = -1;
        if (bundled) {
            dispatch_semaphore_t wait = dispatch_semaphore_create(0);
            [UNUserNotificationCenter.currentNotificationCenter getNotificationSettingsWithCompletionHandler:^(UNNotificationSettings *settings) {
                auth = settings.authorizationStatus; dispatch_semaphore_signal(wait);
            }];
            dispatch_semaphore_wait(wait, dispatch_time(DISPATCH_TIME_NOW, 3*NSEC_PER_SEC));
        }
        NSInteger login = -1;
        if (bundled) {
            if (@available(macOS 13.0, *)) login = SMAppService.mainAppService.status;
        }
        NSDateComponents *parts = [NSCalendar.currentCalendar components:(NSCalendarUnitYear|NSCalendarUnitMonth|NSCalendarUnitDay|NSCalendarUnitWeekday|NSCalendarUnitHour|NSCalendarUnitMinute) fromDate:NSDate.date];
        return json(@{
            @"notifications":@(auth),
            @"login":@(login),
            @"accessibility":@(preelock_accessibility_status()),
            @"day":@((parts.weekday+6)%7),
            @"date":[NSString stringWithFormat:@"%ld-%ld-%ld",(long)parts.year,(long)parts.month,(long)parts.day],
            @"time":[NSString stringWithFormat:@"%02ld:%02ld",(long)parts.hour,(long)parts.minute]
        });
    }
}
char *preelock_login(int enabled) {
    @autoreleasepool {
        __block NSString *message = @"";
        onMain(^{
            if (@available(macOS 13.0, *)) {
                // SMAppService needs a proper .app bundle. Registering the raw
                // dev binary makes macOS launch it via Terminal at login with
                // a white screen. Refuse to register in dev mode, but always
                // allow unregister so a bad registration can be cleaned up.
                BOOL bundled = NSBundle.mainBundle.bundleIdentifier != nil &&
                    [NSBundle.mainBundle.bundleURL.pathExtension.lowercaseString isEqualToString:@"app"];
                if (enabled && !bundled) {
                    message = @"Open at login is available in the installed app.";
                    return;
                }
                NSError *error = nil;
                if (enabled) [SMAppService.mainAppService registerAndReturnError:&error];
                else [SMAppService.mainAppService unregisterAndReturnError:&error];
                if (error) message = error.localizedDescription;
            } else message = @"Open at login needs macOS 13 or later.";
        });
        return strdup(message.UTF8String);
    }
}
int preelock_activate_existing_gui(void);
int preelock_request_gui_activation(const char *source, const char *correlation);
void preelock_install_notification_delegate(void);

int preelock_notification_permission(void) {
    // UNUserNotificationCenter crashes without an .app bundle (dev mode).
    BOOL bundled = NSBundle.mainBundle.bundleIdentifier != nil &&
        [NSBundle.mainBundle.bundleURL.pathExtension.lowercaseString isEqualToString:@"app"];
    if (!bundled) return 1;
    __block int status = 0;
    dispatch_semaphore_t wait = dispatch_semaphore_create(0);
    [UNUserNotificationCenter.currentNotificationCenter getNotificationSettingsWithCompletionHandler:^(UNNotificationSettings *settings) {
        if (settings.authorizationStatus == UNAuthorizationStatusAuthorized ||
            settings.authorizationStatus == UNAuthorizationStatusProvisional) {
            status = 1;
            dispatch_semaphore_signal(wait);
        } else if (settings.authorizationStatus == UNAuthorizationStatusDenied) {
            status = 0;
            dispatch_semaphore_signal(wait);
        } else {
            UNAuthorizationOptions options = UNAuthorizationOptionAlert | UNAuthorizationOptionSound | UNAuthorizationOptionBadge;
            [UNUserNotificationCenter.currentNotificationCenter requestAuthorizationWithOptions:options completionHandler:^(BOOL allowed, NSError *error) {
                status = (allowed && !error) ? 1 : 0;
                dispatch_semaphore_signal(wait);
            }];
        }
    }];
    dispatch_semaphore_wait(wait, dispatch_time(DISPATCH_TIME_NOW, 15*NSEC_PER_SEC));
    return status;
}
void preelock_request_notification_authorization(void) {
    BOOL bundled = NSBundle.mainBundle.bundleIdentifier != nil &&
        [NSBundle.mainBundle.bundleURL.pathExtension.lowercaseString isEqualToString:@"app"];
    if (!bundled) return;
    UNAuthorizationOptions options = UNAuthorizationOptionAlert | UNAuthorizationOptionSound | UNAuthorizationOptionBadge;
    [UNUserNotificationCenter.currentNotificationCenter requestAuthorizationWithOptions:options completionHandler:^(BOOL allowed, NSError *error) {
        (void)allowed;
        if (error) {
            NSLog(@"Preelock notification authorization error: %@", error.localizedDescription);
        }
    }];
}
// Persist the click context (no user content) so a cold-launched GUI can
// consume it exactly once. A click must never depend on a process that is
// still running: this file, plus the correlation id in the launched
// process's environment, is the whole handoff.
static void vyWritePendingNotificationClick(NSString *correlation, NSString *requestId, NSString *actionId, NSString *kind) {
    NSDictionary *payload = @{
        @"correlation": correlation ?: @"",
        @"request_identifier": requestId ?: @"",
        @"action_identifier": actionId ?: @"",
        @"kind": kind ?: @"general",
        @"received_at_ms": @((long long)(NSDate.date.timeIntervalSince1970 * 1000.0)),
    };
    NSString *dir = vyDataDirectory();
    [NSFileManager.defaultManager createDirectoryAtPath:dir withIntermediateDirectories:YES attributes:nil error:nil];
    NSString *path = [dir stringByAppendingPathComponent:@"notification-click.json"];
    NSString *tmp = [path stringByAppendingString:@".tmp"];
    NSData *data = [NSJSONSerialization dataWithJSONObject:payload options:0 error:nil];
    if (data == nil) return;
    if (![data writeToFile:tmp atomically:YES]) return;
    [NSFileManager.defaultManager removeItemAtPath:path error:nil];
    NSError *moveError = nil;
    if (![NSFileManager.defaultManager moveItemAtPath:tmp toPath:path error:&moveError]) {
        vyLifecycleLog(@"could not persist notification click: %@", moveError.localizedDescription);
    }
}

@interface VYAlerts : NSObject <UNUserNotificationCenterDelegate>
@end
@implementation VYAlerts
- (void)userNotificationCenter:(UNUserNotificationCenter *)center willPresentNotification:(UNNotification *)notification withCompletionHandler:(void (^)(UNNotificationPresentationOptions options))completionHandler {
    (void)center; (void)notification;
    completionHandler(UNNotificationPresentationOptionBanner | UNNotificationPresentationOptionList | UNNotificationPresentationOptionSound);
}
// The response is handled in whatever process owns the delegate — the GUI when
// it is alive, otherwise the background engine or tray that posted the
// notification. `preelock_request_gui_activation` is the ONE launch/activate
// entry point; it never relies on LaunchServices re-resolving this bundle to a
// headless instance.
- (void)userNotificationCenter:(UNUserNotificationCenter *)center didReceiveNotificationResponse:(UNNotificationResponse *)response withCompletionHandler:(void (^)(void))completionHandler {
    (void)center;
    NSString *correlation = NSUUID.UUID.UUIDString;
    UNNotificationRequest *request = response.notification.request;
    NSString *kind = request.content.userInfo[@"preelock_kind"];
    if (![kind isKindOfClass:[NSString class]] || kind.length == 0) kind = @"general";
    vyLifecycleLog(@"event=notification_click pid=%d correlation=%@ request=%@ action=%@ kind=%@",
                   getpid(), correlation, request.identifier, response.actionIdentifier, kind);
    vyWritePendingNotificationClick(correlation, request.identifier, response.actionIdentifier, kind);
    preelock_request_gui_activation("notification", correlation.UTF8String);
    // Always complete the response: a click must never leave macOS waiting and
    // must never block the notification centre.
    if (completionHandler) {
        completionHandler();
    }
}
@end

// Install the delegate as early as possible in every bundled process that can
// be running when a notification is clicked (GUI, background engine, tray).
// The center queues responses until a delegate exists, so a cold launch still
// receives the click that caused it.
void preelock_install_notification_delegate(void) {
    if (!vyIsBundledApp()) return;
    static VYAlerts *delegate;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        delegate = [VYAlerts new];
        UNUserNotificationCenter.currentNotificationCenter.delegate = delegate;
        vyLifecycleLog(@"event=notification_delegate_installed pid=%d", getpid());
    });
}

void preelock_notify_ctx(const char *identifier, const char *kind, const char *title, const char *body) {
    BOOL bundled = NSBundle.mainBundle.bundleIdentifier != nil &&
        [NSBundle.mainBundle.bundleURL.pathExtension.lowercaseString isEqualToString:@"app"];
    if (!bundled) {
        // Fallback for development/unbundled test mode: trigger macOS notification via osascript
        @autoreleasepool {
            NSString *nsTitle = [NSString stringWithUTF8String:title ? title : "Preelock"];
            NSString *nsBody = [NSString stringWithUTF8String:body ? body : ""];
            NSString *escapedTitle = [nsTitle stringByReplacingOccurrencesOfString:@"\"" withString:@"\\\""];
            NSString *escapedBody = [nsBody stringByReplacingOccurrencesOfString:@"\"" withString:@"\\\""];
            NSString *script = [NSString stringWithFormat:@"display notification \"%@\" with title \"%@\" sound name \"default\"", escapedBody, escapedTitle];
            NSTask *task = [NSTask new];
            task.launchPath = @"/usr/bin/osascript";
            task.arguments = @[@"-e", script];
            [task launch];
        }
        return;
    }
    preelock_install_notification_delegate();
    UNMutableNotificationContent *content = [UNMutableNotificationContent new];
    content.title = [NSString stringWithUTF8String:title ? title : "Preelock"];
    content.body = [NSString stringWithUTF8String:body ? body : ""];
    content.sound = [UNNotificationSound defaultSound];
    // Only a non-sensitive kind travels with the request (never the text), so
    // a cold launch can tell what the click was about without logging content.
    NSString *kindString = (kind && kind[0]) ? [NSString stringWithUTF8String:kind] : @"general";
    content.userInfo = @{ @"preelock_kind": kindString };
    content.threadIdentifier = @"com.preelock.desktop.notifications";
    // A caller-supplied identifier makes one reminder replace itself instead of
    // stacking a duplicate banner; a missing one falls back to a unique id.
    NSString *requestId = (identifier && identifier[0]) ? [NSString stringWithUTF8String:identifier] : NSUUID.UUID.UUIDString;
    UNNotificationRequest *request = [UNNotificationRequest requestWithIdentifier:requestId content:content trigger:nil];
    [UNUserNotificationCenter.currentNotificationCenter addNotificationRequest:request withCompletionHandler:^(NSError *error) {
        if (error) {
            NSLog(@"Preelock notification unavailable: %@", error.localizedDescription);
        }
    }];
}
void preelock_sound(float volume) {
    if (volume <= 0.0f) return;
    dispatch_block_t play = ^{
        @autoreleasepool {
            NSSound *sound = [NSSound soundNamed:@"Glass"];
            if (!sound) return;
            [sound stop];
            sound.volume = volume;
            [sound play];
        }
    };
    if ([NSThread isMainThread]) {
        play();
    } else if (NSApp != nil && [NSApp isRunning]) {
        dispatch_async(dispatch_get_main_queue(), play);
    } else {
        play();
    }
}
char *preelock_file_panel(int save, const char *filename) {
    @autoreleasepool {
        __block NSString *path = @"";
        NSString *name = [NSString stringWithUTF8String:filename];
        onMain(^{
            [NSApp activateIgnoringOtherApps:YES];
            if (save) {
                NSSavePanel *panel = NSSavePanel.savePanel;
                panel.nameFieldStringValue = name;
                panel.canCreateDirectories = YES;
                if ([panel runModal] == NSModalResponseOK) path = panel.URL.path;
            } else {
                NSOpenPanel *panel = NSOpenPanel.openPanel;
                panel.canChooseDirectories = NO; panel.allowsMultipleSelection = NO;
                panel.message = @"Choose a Preelock backup (.json).";
                if ([panel runModal] == NSModalResponseOK) path = panel.URL.path;
            }
        });
        return strdup(path.UTF8String);
    }
}

// AppKit's terminate: path bypasses Tauri's ExitRequested on macOS.
static void (*vyQuitRequest)(void);
static void (*vySystemShutdown)(void);
static volatile int vyTerminationApproved = 0;

void preelock_approve_quit(void) {
    vyTerminationApproved = 1;
    onMain(^{
        if (NSApp) {
            [NSApp terminate:nil];
        }
    });
}

static NSApplicationTerminateReply vyShouldTerminate(id self, SEL selector, NSApplication *sender) {
    (void)self; (void)selector; (void)sender;
    // If termination was approved by the authoritative quit handler, exit immediately.
    if (vyTerminationApproved) {
        return NSTerminateNow;
    }
    // Let macOS shut down/log out. SQLite checkpoints retain unfinished work.
    NSAppleEventDescriptor *event = NSAppleEventManager.sharedAppleEventManager.currentAppleEvent;
    OSType reason = [[event attributeDescriptorForKeyword:kAEQuitReason] typeCodeValue];
    if (reason == kAELogOut || reason == kAEShutDown || reason == kAERestart) {
        if (vySystemShutdown) vySystemShutdown();
        return NSTerminateNow;
    }
    if (vyQuitRequest) vyQuitRequest();
    return NSTerminateCancel;
}
int preelock_install_quit_guard(void (*callback)(void), void (*shutdown)(void)) {
    __block BOOL installed = NO;
    onMain(^{
        Class delegateClass = [NSApp.delegate class];
        if (!delegateClass) return;
        // Add only our missing delegate method; never replace Tauri's delegate.
        installed = class_addMethod(delegateClass, @selector(applicationShouldTerminate:),
                                    (IMP)vyShouldTerminate, "Q@:@");
        if (installed) { vyQuitRequest = callback; vySystemShutdown = shutdown; }
    });
    return installed;
}

// M2: a tray-only process (Preelock --tray) must not show in the Dock
// or Cmd-Tab — it is a menu-bar icon only. Accessory policy keeps the
// NSStatusItem working while removing the Dock tile.
void preelock_hide_from_dock(void) {
    onMain(^{
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
    });
}

// NOTE: there is deliberately no "set prohibited activation policy" call any
// more. `[NSApplication sharedApplication]` is what registered a second
// com.preelock.desktop instance (BackgroundOnly) for the headless engine in the
// first place, which is the defect this file works around. A process that never
// creates an NSApplication never appears to LaunchServices as the application,
// so macOS cannot activate a daemon instead of the GUI.

void preelock_pump_runloop(double seconds) {
    @autoreleasepool {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, seconds, false);
    }
}

// Bring the Preelock GUI application to the foreground over all other apps.
void preelock_activate_app(void) {
    onMain(^{
        if (NSApp) {
            [NSApp unhide:nil];
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
            [NSApp activateIgnoringOtherApps:YES];
#pragma clang diagnostic pop
        }
    });
}

// True only for a process that really is this bundle's GUI: same bundle id,
// same bundled executable, not terminated, and a Regular activation policy
// (a window-owning app). A headless mode can never satisfy this now, and a
// stale PID that the OS reused is rejected by the identity checks.
static BOOL vyIsOurGuiInstance(NSRunningApplication *app) {
    if (app == nil || app.terminated) return NO;
    NSString *ident = vyBundleIdentifier();
    if (ident.length > 0 && ![app.bundleIdentifier isEqualToString:ident]) return NO;
    if (app.activationPolicy != NSApplicationActivationPolicyRegular) return NO;
    return YES;
}

// The live GUI, if any. gui.lock is a hint (PID + flock), never proof: the PID
// is only trusted when the process exists and is this bundle's GUI.
static NSRunningApplication *vyFindLiveGui(BOOL requireRegular) {
    NSString *lockPath = [vyDataDirectory() stringByAppendingPathComponent:@"gui.lock"];
    NSString *content = [NSString stringWithContentsOfFile:lockPath encoding:NSUTF8StringEncoding error:nil];
    pid_t pid = (pid_t)[content stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet].intValue;
    if (pid > 0 && pid != getpid()) {
        NSRunningApplication *app = [NSRunningApplication runningApplicationWithProcessIdentifier:pid];
        if (app != nil && !app.terminated) {
            BOOL identityOk = [app.bundleIdentifier isEqualToString:vyBundleIdentifier()];
            if (identityOk && (!requireRegular || app.activationPolicy == NSApplicationActivationPolicyRegular)) {
                return app;
            }
            vyLifecycleLog(@"event=stale_gui_lock pid=%d bundle=%@ policy=%ld",
                           pid, app.bundleIdentifier, (long)app.activationPolicy);
        } else {
            vyLifecycleLog(@"event=stale_gui_lock pid=%d alive=0", pid);
        }
    }
    for (NSRunningApplication *app in NSWorkspace.sharedWorkspace.runningApplications) {
        if (app.processIdentifier == getpid()) continue;
        if (vyIsOurGuiInstance(app)) return app;
    }
    return nil;
}

// Activate the existing GUI window. NEVER spawns a new process. Returns 1 when
// an instance was activated, 0 when there is no GUI to activate.
int preelock_activate_existing_gui(void) {
    __block int activated = 0;
    onMain(^{
        NSRunningApplication *gui = vyFindLiveGui(NO);
        if (gui == nil) return;
        [gui unhide];
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
        [gui activateWithOptions:NSApplicationActivateIgnoringOtherApps];
#pragma clang diagnostic pop
        // `applicationShouldHandleReopen` restores a hidden/closed window even
        // for an instance that is still starting up.
        pid_t pid = gui.processIdentifier;
        AEAddressDesc target;
        if (AECreateDesc(typeKernelProcessID, &pid, sizeof(pid), &target) == noErr) {
            AppleEvent reopenEvent;
            if (AECreateAppleEvent(kCoreEventClass, kAEReopenApplication, &target, kAutoGenerateReturnID, kAnyTransactionID, &reopenEvent) == noErr) {
                AESendMessage(&reopenEvent, NULL, kAENoReply, kAEDefaultTimeout);
                AEDisposeDesc(&reopenEvent);
            }
            AEDisposeDesc(&target);
        }
        vyLifecycleLog(@"event=gui_activated pid=%d", pid);
        activated = 1;
    });
    return activated;
}

// THE single GUI entry point for every process mode. Returns:
//   3 = this process is the GUI (a click arrived while it owns gui.lock; the
//       startup path consumes notification-click.json and shows the window),
//   1 = an existing GUI was activated,
//   2 = a fresh GUI was launched through LaunchServices,
//   0 = nothing could be done (not inside an .app bundle, launch failed).
//
// Only one mechanism launches the GUI. LaunchServices is asked for
// `createsNewApplicationInstance` because another mode of this bundle (a tray
// icon) may be the running instance; that cannot produce two GUIs because
// gui.lock is acquired before any AppKit initialization and a loser exits
// without ever touching AppKit.
int preelock_request_gui_activation(const char *source, const char *correlation) {
    __block int result = 0;
    NSString *corr = (correlation && correlation[0]) ? [NSString stringWithUTF8String:correlation] : @"";
    NSString *sourceString = (source && source[0]) ? [NSString stringWithUTF8String:source] : @"unknown";
    NSString *lockPath = [vyDataDirectory() stringByAppendingPathComponent:@"gui.lock"];
    NSString *lockContent = [NSString stringWithContentsOfFile:lockPath encoding:NSUTF8StringEncoding error:nil];
    pid_t lockPid = (pid_t)[lockContent stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet].intValue;
    if (lockPid == getpid()) {
        vyLifecycleLog(@"event=gui_request result=self source=%@ correlation=%@", sourceString, corr);
        return 3;
    }
    NSURL *bundleURL = NSBundle.mainBundle.bundleURL;
    if (![bundleURL.pathExtension.lowercaseString isEqualToString:@"app"]) bundleURL = nil;
    onMain(^{
        if (preelock_activate_existing_gui()) {
            result = 1;
            return;
        }
        if (bundleURL == nil) {
            vyLifecycleLog(@"event=gui_request result=unbundled source=%@ correlation=%@", sourceString, corr);
            return;
        }
        NSMutableDictionary<NSString *, NSString *> *environment =
            [NSProcessInfo.processInfo.environment mutableCopy];
        environment[@"PREELOCK_LAUNCH_SOURCE"] = sourceString;
        if (corr.length > 0) environment[@"PREELOCK_CORRELATION_ID"] = corr;
        // LaunchServices is asked through /usr/bin/open — the documented client
        // for it — rather than an in-process NSWorkspace request, because a
        // short-lived caller (--open-gui) exits before an asynchronous request
        // is serviced and the GUI then never appears. `open -n` is used ONLY
        // when another non-GUI instance of this bundle is already running (a
        // tray icon), where a plain `open -a` would activate that instance
        // instead of creating the GUI. gui.lock, taken before AppKit exists,
        // guarantees the extra process can never become a second GUI: a loser
        // activates the winner and exits without an NSApplication.
        BOOL otherInstanceRunning = NO;
        for (NSRunningApplication *app in NSWorkspace.sharedWorkspace.runningApplications) {
            if (app.processIdentifier != getpid() &&
                [app.bundleIdentifier isEqualToString:vyBundleIdentifier()]) {
                otherInstanceRunning = YES;
                break;
            }
        }
        NSMutableArray<NSString *> *arguments = [NSMutableArray arrayWithObjects:@"-a", bundleURL.path, nil];
        if (otherInstanceRunning) [arguments insertObject:@"-n" atIndex:0];
        vyLifecycleLog(@"event=gui_launch bundle=%@ source=%@ correlation=%@ new_instance=%d",
                       bundleURL.path, sourceString, corr, otherInstanceRunning);
        NSTask *task = [NSTask new];
        task.executableURL = [NSURL fileURLWithPath:@"/usr/bin/open"];
        task.arguments = arguments;
        task.environment = environment;
        task.standardOutput = [NSFileHandle fileHandleWithNullDevice];
        task.standardError = [NSFileHandle fileHandleWithNullDevice];
        NSError *spawnError = nil;
        if (![task launchAndReturnError:&spawnError]) {
            vyLifecycleLog(@"event=gui_launch_failed correlation=%@ error=%@",
                           corr,
                           spawnError.localizedDescription ?: @"could not spawn open");
            return;
        }
        [task waitUntilExit];
        if (task.terminationStatus != 0) {
            vyLifecycleLog(@"event=gui_launch_failed correlation=%@ status=%d",
                           corr,
                           (int)task.terminationStatus);
            return;
        }
        vyLifecycleLog(@"event=gui_launch_completed correlation=%@", corr);
        result = 2;
    });
    return result;
}

// Screen Time row icons: the app's real icon as a base64 PNG string
// (96px, so a 30-row list stays small), or NULL when no icon resolves.
// `identity` is a bundle id (e.g. com.brave.Browser) or a file path
// (bare executables such as the dev binary). A path inside an .app
// bundle resolves to the bundle itself so the bundle icon is returned.
// AppKit work runs on the main thread, like the other preelock_* calls.
char *preelock_app_icon(const char *identity) {
    @autoreleasepool {
        if (!identity || !identity[0]) return NULL;
        __block char *result = NULL;
        onMain(^{
            NSString *ident = [NSString stringWithUTF8String:identity];
            NSString *path = nil;
            if ([ident containsString:@"/"]) {
                NSRange appRange = [ident rangeOfString:@".app/" options:NSBackwardsSearch];
                if (appRange.location != NSNotFound) {
                    path = [ident substringToIndex:appRange.location + 4];
                } else {
                    path = ident;
                }
            } else {
                path = [NSWorkspace.sharedWorkspace URLForApplicationWithBundleIdentifier:ident].path;
            }
            if (!path.length) return;
            if (![NSFileManager.defaultManager fileExistsAtPath:path]) return;
            NSImage *icon = [NSWorkspace.sharedWorkspace iconForFile:path];
            if (!icon) return;
            // Shrink to 96px (retina-sharp at the 40pt row tile) so the
            // PNG stays small instead of shipping the full-size icon.
            NSImage *small = [[NSImage alloc] initWithSize:NSMakeSize(96, 96)];
            [small lockFocus];
            [icon drawInRect:NSMakeRect(0, 0, 96, 96)
                    fromRect:NSZeroRect
                   operation:NSCompositingOperationSourceOver
                    fraction:1.0
              respectFlipped:YES
                       hints:nil];
            [small unlockFocus];
            NSData *tiff = small.TIFFRepresentation;
            if (!tiff) return;
            NSBitmapImageRep *rep = [NSBitmapImageRep imageRepWithData:tiff];
            if (!rep) return;
            NSData *png = [rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
            if (!png.length) return;
            result = strdup([png base64EncodedStringWithOptions:0].UTF8String);
        });
        return result;
    }
}
