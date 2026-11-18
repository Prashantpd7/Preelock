// Read-only integration probe. Does not request permission, hide apps or open a camera.
#import <Foundation/Foundation.h>
extern char *preelock_apps(void); extern char *preelock_sample(void); extern int preelock_camera_permission(void); extern void preelock_free(char *);
extern int preelock_gui_request_allows_cold_launch(const char *);
int main(void) {
    @autoreleasepool {
        if (preelock_gui_request_allows_cold_launch("notification") ||
            !preelock_gui_request_allows_cold_launch("supervisor") ||
            !preelock_gui_request_allows_cold_launch("cli")) return 1;
        char *apps=preelock_apps(), *sample=preelock_sample();
        NSArray *list=[NSJSONSerialization JSONObjectWithData:[[NSString stringWithUTF8String:apps] dataUsingEncoding:NSUTF8StringEncoding] options:0 error:nil];
        NSDictionary *state=[NSJSONSerialization JSONObjectWithData:[[NSString stringWithUTF8String:sample] dataUsingEncoding:NSUTF8StringEncoding] options:0 error:nil];
        preelock_free(apps);preelock_free(sample);
        if (![list isKindOfClass:NSArray.class] || ![state[@"idle_ms"] isKindOfClass:NSNumber.class] || ![state[@"pid"] isKindOfClass:NSNumber.class]) return 1;
        printf("PASS: running-app enumeration, foreground sample, idle API; camera authorization status=%d. No capture or restrictions activated.\n",preelock_camera_permission());
    } return 0;
}
