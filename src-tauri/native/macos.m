#import <AppKit/AppKit.h>
#import <AVFoundation/AVFoundation.h>
#import <Vision/Vision.h>
#import <CoreGraphics/CoreGraphics.h>
#import <mach/mach_time.h>
#import <objc/runtime.h>
#import <Carbon/Carbon.h>

static void onMain(dispatch_block_t work) {
    if ([NSThread isMainThread]) work(); else dispatch_sync(dispatch_get_main_queue(), work);
}
static char *json(id object) {
    NSData *data = [NSJSONSerialization dataWithJSONObject:object options:0 error:nil];
    return strdup([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding].UTF8String ?: "null");
}
char *vy_apps(void) {
    @autoreleasepool {
        __block char *result;
        onMain(^{
            NSMutableArray *apps = [NSMutableArray array];
            for (NSRunningApplication *app in NSWorkspace.sharedWorkspace.runningApplications) {
                if (app.activationPolicy == NSApplicationActivationPolicyRegular && app.bundleIdentifier)
                    [apps addObject:@{@"id":app.bundleIdentifier, @"name":app.localizedName ?: app.bundleIdentifier}];
            }
            result = json(apps);
        });
        return result;
    }
}
char *vy_sample(void) {
    @autoreleasepool {
        __block char *result;
        onMain(^{
            NSRunningApplication *app = NSWorkspace.sharedWorkspace.frontmostApplication;
            result = json(@{@"id": app.bundleIdentifier ?: @"", @"name":app.localizedName ?: @"", @"pid":@(app.processIdentifier), @"idle_ms":@((uint64_t)(CGEventSourceSecondsSinceLastEventType(kCGEventSourceStateCombinedSessionState, kCGAnyInputEventType)*1000))});
        });
        return result;
    }
}
int vy_hide(int pid) {
    __block BOOL result = NO;
    onMain(^{ NSRunningApplication *app = [NSRunningApplication runningApplicationWithProcessIdentifier:pid]; if (app.activationPolicy == NSApplicationActivationPolicyRegular) result = [app hide]; });
    return result;
}
int vy_camera_permission(void) { return (int)[AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo]; }
void vy_request_camera(void) { [AVCaptureDevice requestAccessForMediaType:AVMediaTypeVideo completionHandler:^(BOOL granted) { (void)granted; }]; }

@interface VYCamera : NSObject <AVCaptureVideoDataOutputSampleBufferDelegate>
@property AVCaptureSession *session;
@property dispatch_queue_t queue;
@property NSString *status;
@property double lastFrame;
@property double lastSeen;
@end
@implementation VYCamera
- (instancetype)init { if ((self=[super init])) { _queue=dispatch_queue_create("com.vystriq.camera", DISPATCH_QUEUE_SERIAL); _status=@"off"; } return self; }
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
int vy_camera_start(void) {
    if (vy_camera_permission()!=AVAuthorizationStatusAuthorized) return 0;
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
void vy_camera_stop(void) {
    VYCamera *c=getCamera(); dispatch_sync(c.queue, ^{ [c.session stopRunning]; c.session=nil; c.status=@"off"; });
}
char *vy_camera_status(void) {
    VYCamera *c=getCamera(); __block char *result;
    dispatch_sync(c.queue, ^{
        NSString *status=c.status;
        if (c.session && !c.session.isRunning) status=@"camera_interrupted";
        else if (c.session && NSProcessInfo.processInfo.systemUptime-(c.lastFrame>0 ? c.lastFrame : c.lastSeen)>10) status=@"camera_stalled";
        result=strdup(status.UTF8String);
    }); return result;
}
void vy_free(char *data) { free(data); }

// Settings integrations. All panels and AppKit work stay on the main thread.
#import <UserNotifications/UserNotifications.h>
#import <ServiceManagement/ServiceManagement.h>
char *vy_settings_info(void) {
    @autoreleasepool {
        __block NSInteger auth = 0;
        dispatch_semaphore_t wait = dispatch_semaphore_create(0);
        [UNUserNotificationCenter.currentNotificationCenter getNotificationSettingsWithCompletionHandler:^(UNNotificationSettings *settings) {
            auth = settings.authorizationStatus; dispatch_semaphore_signal(wait);
        }];
        dispatch_semaphore_wait(wait, dispatch_time(DISPATCH_TIME_NOW, 3*NSEC_PER_SEC));
        NSInteger login = -1;
        if (@available(macOS 13.0, *)) login = SMAppService.mainAppService.status;
        NSDateComponents *parts = [NSCalendar.currentCalendar components:(NSCalendarUnitYear|NSCalendarUnitMonth|NSCalendarUnitDay|NSCalendarUnitWeekday|NSCalendarUnitHour|NSCalendarUnitMinute) fromDate:NSDate.date];
        return json(@{@"notifications":@(auth), @"login":@(login), @"day":@((parts.weekday+6)%7), @"date":[NSString stringWithFormat:@"%ld-%ld-%ld",(long)parts.year,(long)parts.month,(long)parts.day], @"time":[NSString stringWithFormat:@"%02ld:%02ld",(long)parts.hour,(long)parts.minute]});
    }
}
char *vy_login(int enabled) {
    @autoreleasepool {
        __block NSString *message = @"";
        onMain(^{
            if (@available(macOS 13.0, *)) {
                NSError *error = nil;
                if (enabled) [SMAppService.mainAppService registerAndReturnError:&error];
                else [SMAppService.mainAppService unregisterAndReturnError:&error];
                if (error) message = error.localizedDescription;
            } else message = @"Open at login needs macOS 13 or later.";
        });
        return strdup(message.UTF8String);
    }
}
int vy_notification_permission(void) {
    __block BOOL granted = NO;
    dispatch_semaphore_t wait = dispatch_semaphore_create(0);
    [UNUserNotificationCenter.currentNotificationCenter requestAuthorizationWithOptions:UNAuthorizationOptionAlert completionHandler:^(BOOL allowed, NSError *error) {
        granted = allowed && !error; dispatch_semaphore_signal(wait);
    }];
    dispatch_semaphore_wait(wait, dispatch_time(DISPATCH_TIME_NOW, 60*NSEC_PER_SEC));
    return granted;
}
@interface VYAlerts : NSObject <UNUserNotificationCenterDelegate>
@end
@implementation VYAlerts
- (void)userNotificationCenter:(UNUserNotificationCenter *)center willPresentNotification:(UNNotification *)notification withCompletionHandler:(void (^)(UNNotificationPresentationOptions options))completionHandler {
    (void)center; (void)notification;
    completionHandler(UNNotificationPresentationOptionBanner | UNNotificationPresentationOptionList);
}
@end
void vy_notify(const char *title, const char *body) {
    static VYAlerts *delegate;
    static dispatch_once_t once;
    dispatch_once(&once, ^{delegate=[VYAlerts new]; UNUserNotificationCenter.currentNotificationCenter.delegate=delegate;});
    UNMutableNotificationContent *content = [UNMutableNotificationContent new];
    content.title = [NSString stringWithUTF8String:title]; content.body = [NSString stringWithUTF8String:body];
    UNNotificationRequest *request = [UNNotificationRequest requestWithIdentifier:NSUUID.UUID.UUIDString content:content trigger:nil];
    [UNUserNotificationCenter.currentNotificationCenter addNotificationRequest:request withCompletionHandler:^(NSError *error) { if (error) NSLog(@"Preelock notification unavailable: %@", error.localizedDescription); }];
}
void vy_sound(float volume) {
    onMain(^{ NSSound *sound = [NSSound soundNamed:@"Glass"]; sound.volume = volume; [sound play]; });
}
char *vy_file_panel(int save, const char *filename) {
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
static NSApplicationTerminateReply vyShouldTerminate(id self, SEL selector, NSApplication *sender) {
    (void)self; (void)selector; (void)sender;
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
int vy_install_quit_guard(void (*callback)(void), void (*shutdown)(void)) {
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
