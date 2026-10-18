// Read-only integration probe. Does not request permission, hide apps or open a camera.
#import <Foundation/Foundation.h>
extern char *vy_apps(void); extern char *vy_sample(void); extern int vy_camera_permission(void); extern void vy_free(char *);
int main(void) {
    @autoreleasepool {
        char *apps=vy_apps(), *sample=vy_sample();
        NSArray *list=[NSJSONSerialization JSONObjectWithData:[[NSString stringWithUTF8String:apps] dataUsingEncoding:NSUTF8StringEncoding] options:0 error:nil];
        NSDictionary *state=[NSJSONSerialization JSONObjectWithData:[[NSString stringWithUTF8String:sample] dataUsingEncoding:NSUTF8StringEncoding] options:0 error:nil];
        vy_free(apps);vy_free(sample);
        if (![list isKindOfClass:NSArray.class] || ![state[@"idle_ms"] isKindOfClass:NSNumber.class] || ![state[@"pid"] isKindOfClass:NSNumber.class]) return 1;
        printf("PASS: running-app enumeration, foreground sample, idle API; camera authorization status=%d. No capture or restrictions activated.\n",vy_camera_permission());
    } return 0;
}
