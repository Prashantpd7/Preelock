#import <AppKit/AppKit.h>
#import <AVFoundation/AVFoundation.h>
#import <Vision/Vision.h>
#import <CoreGraphics/CoreGraphics.h>
#import <mach/mach_time.h>

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
