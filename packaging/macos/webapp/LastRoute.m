// Last Route for macOS: a thin native window around the game's WEB build.
//
// The app is one WKWebView. The page it shows is the same set of files the website serves (index.html, update.js, keys.js, hw.js,
// audio.js, mq_js_bundle.js, last_route.wasm, version.json). They are served to the web view from a per-user folder by a custom
// URL scheme (lrapp://game/), so there is no local HTTP server and no file:// restriction. The app keeps that folder current by
// itself: it asks <site>/version.json, downloads a newer build, checks its sha256, switches to it, and keeps the previous build
// for rollback. It runs offline from the last good copy and falls back to the copy bundled inside the .app.
// See docs/MACOS_WEB_APP.md for the design, the flags and what was verified.
//
// Build: sh packaging/macos/webapp/build-app.sh (clang + the Command Line Tools, no Xcode project). Objective-C on purpose: it
// needs no Swift runtime, so the same binary starts on macOS 10.13 and up and compiles as a universal (x86_64 + arm64) app.

#import <Cocoa/Cocoa.h>
#import <WebKit/WebKit.h>
#import <CommonCrypto/CommonDigest.h>
#include <sys/utsname.h>
#include <unistd.h>

#define APP_VERSION @"1.0.0"
static NSString *const kDefaultSite = @"https://lastrouterust.games.kyttuslab.org";
static NSString *const kScheme = @"lrapp";
static NSString *const kPageURL = @"lrapp://game/index.html";

// ---------------------------------------------------------------------------------------------------------------------
// Settings from the command line / environment
// ---------------------------------------------------------------------------------------------------------------------

static NSString *gSite;               // the website (no trailing slash)
static NSString *gDataDir;            // ~/Library/Application Support/LastRoute
static NSString *gLogPath;            // ~/Library/Logs/LastRoute/app.log
static BOOL gSelfTest = NO;           // --self-test: boot headless-ish, probe the page, write the log, exit
static BOOL gSelfTestVisible = NO;    // --self-test-visible: the same with a normal, focused window (needed to test pointer lock)
static BOOL gNoUpdate = NO;           // --no-update: never ask the site (the offline test)
static double gReadyTimeout = 90;     // seconds the page gets to report ready before the boot counts as failed
static double gCheckInterval = 600;   // seconds between update checks while running
static double gSelfTestHold = 0;      // --self-test-hold S: after the probes keep running up to S seconds, ending early when the page reloads itself
static double gFirstCheckDelay = 5;   // seconds after start before the first check
static int gMaxFailures = 2;          // failed boots of one build in a row before it is rolled back

static NSArray<NSString *> *LRDefaultFiles(void) {
    return @[ @"index.html", @"update.js", @"keys.js", @"hw.js", @"audio.js", @"mq_js_bundle.js", @"last_route.wasm" ];
}

// ---------------------------------------------------------------------------------------------------------------------
// Log
// ---------------------------------------------------------------------------------------------------------------------

static dispatch_queue_t gLogQueue;
static FILE *gLogFile;
static BOOL gLogEcho = NO;

static void LRLogOpen(void) {
    gLogQueue = dispatch_queue_create("lr.log", DISPATCH_QUEUE_SERIAL);
    NSFileManager *fm = [NSFileManager defaultManager];
    [fm createDirectoryAtPath:[gLogPath stringByDeletingLastPathComponent] withIntermediateDirectories:YES attributes:nil error:nil];
    NSDictionary *attrs = [fm attributesOfItemAtPath:gLogPath error:nil];
    if ([attrs fileSize] > 2 * 1024 * 1024) {
        NSString *old = [gLogPath stringByAppendingString:@".1"];
        [fm removeItemAtPath:old error:nil];
        [fm moveItemAtPath:gLogPath toPath:old error:nil];
    }
    gLogFile = fopen(gLogPath.fileSystemRepresentation, "a");
}

static void LRLog(NSString *format, ...) NS_FORMAT_FUNCTION(1, 2);
static void LRLog(NSString *format, ...) {
    va_list args;
    va_start(args, format);
    NSString *message = [[NSString alloc] initWithFormat:format arguments:args];
    va_end(args);
    NSDate *now = [NSDate date];
    dispatch_async(gLogQueue, ^{
        static NSDateFormatter *formatter;
        if (!formatter) {
            formatter = [NSDateFormatter new];
            formatter.locale = [NSLocale localeWithLocaleIdentifier:@"en_US_POSIX"];
            formatter.dateFormat = @"yyyy-MM-dd HH:mm:ss.SSS";
        }
        const char *line = [[NSString stringWithFormat:@"%@ [%d] %@\n", [formatter stringFromDate:now], getpid(), message] UTF8String];
        if (gLogFile) {
            fputs(line, gLogFile);
            fflush(gLogFile);
        }
        if (gLogEcho) fputs(line, stdout), fflush(stdout);
    });
}

static void LRLogFlush(void) {
    dispatch_sync(gLogQueue, ^{
    });
}

// ---------------------------------------------------------------------------------------------------------------------
// Small helpers: hashes, JSON, file names, MIME types
// ---------------------------------------------------------------------------------------------------------------------

static NSString *LRHex(const unsigned char *bytes, int count) {
    NSMutableString *s = [NSMutableString stringWithCapacity:count * 2];
    for (int i = 0; i < count; i++) [s appendFormat:@"%02x", bytes[i]];
    return s;
}

static NSString *LRSha256Data(NSData *data) {
    unsigned char digest[CC_SHA256_DIGEST_LENGTH];
    CC_SHA256(data.bytes, (CC_LONG)data.length, digest);
    return LRHex(digest, CC_SHA256_DIGEST_LENGTH);
}

static NSString *LRSha256File(NSString *path) {
    FILE *f = fopen(path.fileSystemRepresentation, "rb");
    if (!f) return nil;
    CC_SHA256_CTX ctx;
    CC_SHA256_Init(&ctx);
    size_t cap = 1 << 20;
    unsigned char *buffer = malloc(cap);
    size_t n;
    while ((n = fread(buffer, 1, cap, f)) > 0) CC_SHA256_Update(&ctx, buffer, (CC_LONG)n);
    BOOL failed = ferror(f) != 0;
    free(buffer);
    fclose(f);
    if (failed) return nil;
    unsigned char digest[CC_SHA256_DIGEST_LENGTH];
    CC_SHA256_Final(digest, &ctx);
    return LRHex(digest, CC_SHA256_DIGEST_LENGTH);
}

static unsigned long long LRFileSize(NSString *path) {
    NSDictionary *attrs = [[NSFileManager defaultManager] attributesOfItemAtPath:path error:nil];
    return attrs ? [attrs fileSize] : 0;
}

static id LRReadJSON(NSString *path) {
    NSData *data = [NSData dataWithContentsOfFile:path];
    if (!data) return nil;
    return [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
}

static BOOL LRWriteJSON(id object, NSString *path) {
    NSData *data = [NSJSONSerialization dataWithJSONObject:object options:NSJSONWritingPrettyPrinted error:nil];
    return data && [data writeToFile:path atomically:YES];
}

// A name the shell will serve or download: one path component, no dots at the start, no separators.
static BOOL LRSafeName(NSString *name) {
    if (![name isKindOfClass:[NSString class]] || name.length == 0 || name.length > 80) return NO;
    if ([name hasPrefix:@"."] || [name containsString:@"/"] || [name containsString:@"\\"] || [name containsString:@"\0"]) return NO;
    return YES;
}

static BOOL LRIsHex64(NSString *s) {
    if (![s isKindOfClass:[NSString class]] || s.length != 64) return NO;
    NSCharacterSet *bad = [[NSCharacterSet characterSetWithCharactersInString:@"0123456789abcdef"] invertedSet];
    return [s rangeOfCharacterFromSet:bad].location == NSNotFound;
}

static NSString *LRMime(NSString *name) {
    NSString *ext = name.pathExtension.lowercaseString;
    static NSDictionary *types;
    if (!types) {
        types = @{
            @"html" : @"text/html; charset=utf-8",
            @"js" : @"text/javascript; charset=utf-8",
            @"mjs" : @"text/javascript; charset=utf-8",
            @"wasm" : @"application/wasm",
            @"json" : @"application/json; charset=utf-8",
            @"css" : @"text/css; charset=utf-8",
            @"txt" : @"text/plain; charset=utf-8",
            @"png" : @"image/png",
            @"jpg" : @"image/jpeg",
            @"jpeg" : @"image/jpeg",
            @"svg" : @"image/svg+xml",
            @"ico" : @"image/x-icon",
            @"ogg" : @"audio/ogg",
            @"mp3" : @"audio/mpeg",
            @"wav" : @"audio/wav",
            @"webmanifest" : @"application/manifest+json",
        };
    }
    return types[ext] ?: @"application/octet-stream";
}

// ---------------------------------------------------------------------------------------------------------------------
// Store: the per-user web folders and the pointer to the current one
// ---------------------------------------------------------------------------------------------------------------------
//
//   <data dir>/web/<id>/     one complete copy of the site's files + version.json + meta.json (etags). <id> = first 12 hex of
//                            the wasm sha256, "-", first 8 hex of a hash of the other files, so a changed page file with the
//                            same wasm is a new copy too
//   <data dir>/state.json    the pointer: {current, previous, bad:[ids], badBuilds:[build12], failures:{id:n}, booting:id}

static NSString *const kStateFile = @"state.json";

static BOOL LRFolderComplete(NSString *dir) {
    if (!dir) return NO;
    for (NSString *name in @[ @"index.html", @"last_route.wasm", @"version.json" ]) {
        if (LRFileSize([dir stringByAppendingPathComponent:name]) == 0) return NO;
    }
    return YES;
}

static NSDictionary *LRVersionOf(NSString *dir) {
    id json = LRReadJSON([dir stringByAppendingPathComponent:@"version.json"]);
    return [json isKindOfClass:[NSDictionary class]] ? json : nil;
}

// The id for a folder's content, or nil when version.json has no build.
static NSString *LRComputeId(NSString *dir) {
    NSString *build = LRVersionOf(dir)[@"build"];
    if (![build isKindOfClass:[NSString class]] || build.length < 12) return nil;
    NSArray *names = [[[NSFileManager defaultManager] contentsOfDirectoryAtPath:dir error:nil] sortedArrayUsingSelector:@selector(compare:)];
    NSMutableString *acc = [NSMutableString string];
    for (NSString *name in names) {
        if ([name hasPrefix:@"."] || [name isEqualToString:@"version.json"] || [name isEqualToString:@"meta.json"] ||
            [name isEqualToString:@"last_route.wasm"])
            continue;
        NSString *sha = LRSha256File([dir stringByAppendingPathComponent:name]);
        if (sha) [acc appendFormat:@"%@:%@\n", name, sha];
    }
    NSString *pages = [LRSha256Data([acc dataUsingEncoding:NSUTF8StringEncoding]) substringToIndex:8];
    return [NSString stringWithFormat:@"%@-%@", [build substringToIndex:12], pages];
}

@interface LRStore : NSObject
@property(nonatomic, copy) NSString *webDir;
@property(nonatomic, copy) NSString *bundleDir;  // <app>/Contents/Resources/web, nil when the app has no bundled build
@property(nonatomic, copy) NSString *bundleId;
@property(copy) NSString *pendingId;  // downloaded and ready, switched to when the page is safe to leave
- (void)setup;
- (NSString *)dirForId:(NSString *)ident;
- (NSString *)currentId;
- (NSString *)previousId;
- (NSString *)currentDir;    // the folder to serve; the bundled one straight from the app when nothing is installed
- (NSString *)bootId;        // the id the failure counters use for currentDir
- (NSString *)latestId;      // pending, else current
- (BOOL)isBadId:(NSString *)ident;
- (BOOL)isBadBuild:(NSString *)build;
- (int)bumpFailure:(NSString *)ident;
- (void)clearFailure:(NSString *)ident;
- (void)setBooting:(NSString *)ident;
- (NSString *)bootingId;
- (void)markBad:(NSString *)ident;
- (NSString *)rollbackFrom:(NSString *)ident;
- (BOOL)installStage:(NSString *)stage asId:(NSString *)ident;
- (void)activate:(NSString *)ident;
- (void)resetToBundled;
@end

@implementation LRStore {
    NSMutableDictionary *_state;
}

- (NSString *)statePath {
    return [gDataDir stringByAppendingPathComponent:kStateFile];
}

- (void)save {
    @synchronized(self) {
        LRWriteJSON(_state, [self statePath]);
    }
}

- (NSString *)dirForId:(NSString *)ident {
    return ident.length ? [self.webDir stringByAppendingPathComponent:ident] : nil;
}

- (void)setup {
    NSFileManager *fm = [NSFileManager defaultManager];
    self.webDir = [gDataDir stringByAppendingPathComponent:@"web"];
    [fm createDirectoryAtPath:self.webDir withIntermediateDirectories:YES attributes:nil error:nil];
    id loaded = LRReadJSON([self statePath]);
    _state = [loaded isKindOfClass:[NSDictionary class]] ? [loaded mutableCopy] : [NSMutableDictionary dictionary];
    for (NSString *key in @[ @"bad", @"badBuilds" ]) {
        if (![_state[key] isKindOfClass:[NSArray class]]) _state[key] = @[];
    }
    if (![_state[@"failures"] isKindOfClass:[NSDictionary class]]) _state[@"failures"] = @{};
    // half-finished downloads from a run that was killed
    for (NSString *name in [fm contentsOfDirectoryAtPath:self.webDir error:nil]) {
        if ([name hasPrefix:@".stage"]) [fm removeItemAtPath:[self.webDir stringByAppendingPathComponent:name] error:nil];
    }
    // the bundled build
    NSString *resources = [[NSBundle mainBundle] resourcePath];
    NSString *bundled = [resources stringByAppendingPathComponent:@"web"];
    if (LRFolderComplete(bundled)) {
        self.bundleDir = bundled;
        self.bundleId = LRComputeId(bundled);
    }
    // a current that is gone or damaged is dropped; the previous takes over
    NSString *current = _state[@"current"];
    if (current && !LRFolderComplete([self dirForId:current])) {
        LRLog(@"store: current %@ is missing or incomplete", current);
        current = nil;
        _state[@"current"] = nil;
    }
    NSString *previous = _state[@"previous"];
    if (previous && !LRFolderComplete([self dirForId:previous])) {
        previous = nil;
        _state[@"previous"] = nil;
    }
    if (!current && previous) {
        _state[@"current"] = previous;
        _state[@"previous"] = nil;
        current = previous;
    }
    // first run, or an app update that carries a newer bundled build than the installed one
    if (self.bundleDir && self.bundleId) {
        NSString *bundledBuilt = LRVersionOf(self.bundleDir)[@"built"];
        NSString *currentBuilt = current ? LRVersionOf([self dirForId:current])[@"built"] : nil;
        BOOL newer = current && [bundledBuilt isKindOfClass:[NSString class]] && [currentBuilt isKindOfClass:[NSString class]] &&
                     [bundledBuilt compare:currentBuilt] == NSOrderedDescending && ![self isBadId:self.bundleId];
        if (!current || newer) {
            if ([self copyBundleToWeb]) {
                LRLog(@"store: %@ the bundled build %@", current ? @"adopting the newer" : @"first run, installing", self.bundleId);
                if (current && ![current isEqualToString:self.bundleId]) _state[@"previous"] = current;
                _state[@"current"] = self.bundleId;
            }
        }
    }
    [self pruneKeeping:nil];
    [self save];
}

- (BOOL)copyBundleToWeb {
    if (!self.bundleDir || !self.bundleId) return NO;
    NSString *target = [self dirForId:self.bundleId];
    if (LRFolderComplete(target)) return YES;
    NSFileManager *fm = [NSFileManager defaultManager];
    NSString *stage = [self.webDir stringByAppendingPathComponent:@".stage-bundle"];
    [fm removeItemAtPath:stage error:nil];
    if (![fm copyItemAtPath:self.bundleDir toPath:stage error:nil]) {
        LRLog(@"store: copying the bundled build failed");
        return NO;
    }
    [fm removeItemAtPath:target error:nil];
    return [fm moveItemAtPath:stage toPath:target error:nil];
}

- (NSString *)currentId {
    @synchronized(self) {
        return _state[@"current"];
    }
}

- (NSString *)previousId {
    @synchronized(self) {
        return _state[@"previous"];
    }
}

- (NSString *)currentDir {
    NSString *current = [self currentId];
    NSString *dir = [self dirForId:current];
    if (LRFolderComplete(dir)) return dir;
    return self.bundleDir;
}

- (NSString *)bootId {
    NSString *current = [self currentId];
    if (current && LRFolderComplete([self dirForId:current])) return current;
    return self.bundleId ?: @"bundled";
}

- (NSString *)latestId {
    return self.pendingId ?: [self currentId];
}

- (BOOL)isBadId:(NSString *)ident {
    @synchronized(self) {
        return ident && [_state[@"bad"] containsObject:ident];
    }
}

- (BOOL)isBadBuild:(NSString *)build {
    @synchronized(self) {
        return build.length >= 12 && [_state[@"badBuilds"] containsObject:[build substringToIndex:12]];
    }
}

- (int)bumpFailure:(NSString *)ident {
    @synchronized(self) {
        NSMutableDictionary *f = [_state[@"failures"] mutableCopy];
        int n = [f[ident] intValue] + 1;
        f[ident] = @(n);
        _state[@"failures"] = f;
        [self save];
        return n;
    }
}

- (void)clearFailure:(NSString *)ident {
    @synchronized(self) {
        if (!_state[@"failures"][ident] && !_state[@"booting"]) return;
        NSMutableDictionary *f = [_state[@"failures"] mutableCopy];
        [f removeObjectForKey:ident];
        _state[@"failures"] = f;
        [self save];
    }
}

- (void)setBooting:(NSString *)ident {
    @synchronized(self) {
        _state[@"booting"] = ident;
        [self save];
    }
}

- (NSString *)bootingId {
    @synchronized(self) {
        return _state[@"booting"];
    }
}

- (void)markBad:(NSString *)ident {
    @synchronized(self) {
        NSMutableArray *bad = [_state[@"bad"] mutableCopy];
        if (![bad containsObject:ident]) [bad addObject:ident];
        _state[@"bad"] = bad;
        NSMutableArray *builds = [_state[@"badBuilds"] mutableCopy];
        NSString *b12 = ident.length >= 12 ? [ident substringToIndex:12] : ident;
        if (![builds containsObject:b12]) [builds addObject:b12];
        _state[@"badBuilds"] = builds;
        [self save];
    }
    LRLog(@"store: %@ is marked bad", ident);
}

// Picks what to run after `ident` failed: the previous build, else the bundled one; nil when nothing is left.
- (NSString *)rollbackFrom:(NSString *)ident {
    @synchronized(self) {
        NSString *previous = _state[@"previous"];
        NSString *target = nil;
        if (previous && ![previous isEqualToString:ident] && ![self isBadId:previous] && LRFolderComplete([self dirForId:previous])) {
            target = previous;
        } else if (self.bundleId && ![self.bundleId isEqualToString:ident] && ![self isBadId:self.bundleId] && [self copyBundleToWeb]) {
            target = self.bundleId;
        }
        if (!target) return nil;
        _state[@"current"] = target;
        _state[@"previous"] = nil;
        [self save];
        [self pruneKeeping:nil];
        return target;
    }
}

- (void)resetToBundled {
    @synchronized(self) {
        if (![self copyBundleToWeb]) return;
        _state[@"current"] = self.bundleId;
        _state[@"previous"] = nil;
        _state[@"bad"] = @[];
        _state[@"badBuilds"] = @[];
        _state[@"failures"] = @{};
        self.pendingId = nil;
        [self save];
        [self pruneKeeping:nil];
    }
}

// Moves a finished stage folder into place under its id (replacing an equal one).
- (BOOL)installStage:(NSString *)stage asId:(NSString *)ident {
    NSFileManager *fm = [NSFileManager defaultManager];
    NSString *target = [self dirForId:ident];
    if (!LRFolderComplete(stage)) return NO;
    [fm removeItemAtPath:target error:nil];
    return [fm moveItemAtPath:stage toPath:target error:nil];
}

// The current pointer moves to `ident`; the old current becomes the previous; everything older is deleted.
- (void)activate:(NSString *)ident {
    @synchronized(self) {
        NSString *old = _state[@"current"];
        if ([old isEqualToString:ident]) return;
        if (old && LRFolderComplete([self dirForId:old])) _state[@"previous"] = old;
        _state[@"current"] = ident;
        if ([self.pendingId isEqualToString:ident]) self.pendingId = nil;
        [self save];
        [self pruneKeeping:nil];
    }
}

- (void)pruneKeeping:(NSString *)extra {
    NSFileManager *fm = [NSFileManager defaultManager];
    NSMutableSet *keep = [NSMutableSet set];
    for (NSString *key in @[ @"current", @"previous" ]) {
        if (_state[key]) [keep addObject:_state[key]];
    }
    if (self.pendingId) [keep addObject:self.pendingId];
    if (extra) [keep addObject:extra];
    for (NSString *name in [fm contentsOfDirectoryAtPath:self.webDir error:nil]) {
        if ([name hasPrefix:@"."] || [keep containsObject:name]) continue;
        LRLog(@"store: deleting old build %@", name);
        [fm removeItemAtPath:[self.webDir stringByAppendingPathComponent:name] error:nil];
    }
}

@end

static LRStore *gStore;

// ---------------------------------------------------------------------------------------------------------------------
// Scheme handler: lrapp://game/<file> from the current folder, lrapp://game/api/* forwarded to the site
// ---------------------------------------------------------------------------------------------------------------------

@interface LRSchemeHandler : NSObject <WKURLSchemeHandler>
@property(nonatomic, copy) NSString *pinnedDir;  // the folder one page load reads all its files from
@end

@implementation LRSchemeHandler {
    NSMutableSet *_active;
    NSString *_lastVersionServed;
    NSURLSession *_proxy;
}

- (instancetype)init {
    if ((self = [super init])) {
        _active = [NSMutableSet set];
        NSURLSessionConfiguration *c = [NSURLSessionConfiguration ephemeralSessionConfiguration];
        c.timeoutIntervalForRequest = 30;
        _proxy = [NSURLSession sessionWithConfiguration:c];
    }
    return self;
}

- (void)reply:(id<WKURLSchemeTask>)task status:(NSInteger)status headers:(NSDictionary *)headers body:(NSData *)body {
    if (![_active containsObject:task]) return;
    [_active removeObject:task];
    @try {
        NSHTTPURLResponse *response = [[NSHTTPURLResponse alloc] initWithURL:task.request.URL
                                                                  statusCode:status
                                                                 HTTPVersion:@"HTTP/1.1"
                                                                headerFields:headers];
        [task didReceiveResponse:response];
        if (body.length > 0) [task didReceiveData:body];
        [task didFinish];
    } @catch (NSException *e) {
        LRLog(@"scheme: reply failed: %@", e.reason);
    }
}

- (void)replyText:(id<WKURLSchemeTask>)task status:(NSInteger)status text:(NSString *)text {
    NSData *body = [text dataUsingEncoding:NSUTF8StringEncoding];
    [self reply:task
         status:status
        headers:@{
            @"Content-Type" : @"text/plain; charset=utf-8",
            @"Content-Length" : [NSString stringWithFormat:@"%lu", (unsigned long)body.length],
            @"Cache-Control" : @"no-store"
        }
           body:body];
}

- (void)webView:(WKWebView *)webView startURLSchemeTask:(id<WKURLSchemeTask>)task {
    [_active addObject:task];
    NSURLRequest *request = task.request;
    NSString *path = request.URL.path ?: @"/";
    if ([path isEqualToString:@"/api"] || [path hasPrefix:@"/api/"]) {
        [self forward:task];
        return;
    }
    NSString *method = request.HTTPMethod ?: @"GET";
    if (![method isEqualToString:@"GET"] && ![method isEqualToString:@"HEAD"]) {
        [self replyText:task status:405 text:@"method not allowed"];
        return;
    }
    NSString *name = path;
    while ([name hasPrefix:@"/"]) name = [name substringFromIndex:1];
    if (name.length == 0) name = @"index.html";
    if (!LRSafeName(name)) {
        [self replyText:task status:400 text:@"bad path"];
        return;
    }
    NSString *dir;
    if ([name isEqualToString:@"version.json"]) {
        // the page's update.js reads this to learn whether the files it runs are still the current ones
        dir = [gStore currentDir];
        if (![dir.lastPathComponent isEqualToString:_lastVersionServed]) {
            _lastVersionServed = dir.lastPathComponent;
            LRLog(@"scheme: version.json is now served from %@", _lastVersionServed);
        }
    } else {
        if ([name isEqualToString:@"index.html"]) {
            self.pinnedDir = [gStore currentDir];
            LRLog(@"scheme: page load pinned to %@", self.pinnedDir.lastPathComponent);
        }
        dir = self.pinnedDir ?: [gStore currentDir];
    }
    NSString *file = dir ? [dir stringByAppendingPathComponent:name] : nil;
    BOOL isDir = NO;
    NSData *data = nil;
    if (file && [[NSFileManager defaultManager] fileExistsAtPath:file isDirectory:&isDir] && !isDir) {
        data = [NSData dataWithContentsOfFile:file options:NSDataReadingMappedIfSafe error:nil];
    }
    if (!data) {
        [self replyText:task status:404 text:@"not found"];
        return;
    }
    NSInteger status = 200;
    NSMutableDictionary *headers = [@{
        @"Content-Type" : LRMime(name),
        @"Cache-Control" : @"no-store",
        @"Accept-Ranges" : @"bytes",
        @"Access-Control-Allow-Origin" : @"*",
    } mutableCopy];
    NSString *range = [request valueForHTTPHeaderField:@"Range"];
    if ([range hasPrefix:@"bytes="]) {
        NSArray *parts = [[range substringFromIndex:6] componentsSeparatedByString:@"-"];
        long long total = (long long)data.length, first = 0, last = total - 1;
        if (parts.count == 2) {
            NSString *a = parts[0], *b = parts[1];
            if (a.length && b.length) {
                first = a.longLongValue;
                last = b.longLongValue;
            } else if (a.length) {
                first = a.longLongValue;
            } else if (b.length) {
                first = MAX(0, total - b.longLongValue);
            }
            if (last >= total) last = total - 1;
            if (first <= last && first < total) {
                data = [data subdataWithRange:NSMakeRange((NSUInteger)first, (NSUInteger)(last - first + 1))];
                status = 206;
                headers[@"Content-Range"] = [NSString stringWithFormat:@"bytes %lld-%lld/%lld", first, last, total];
            }
        }
    }
    headers[@"Content-Length"] = [NSString stringWithFormat:@"%lu", (unsigned long)data.length];
    if ([method isEqualToString:@"HEAD"]) data = nil;
    [self reply:task status:status headers:headers body:data];
}

// /api/* -> <site>/api/*: the same method, headers and body; the page normally talks to the site directly (LR_API_BASE) and this
// covers anything that asks the page's own origin.
- (void)forward:(id<WKURLSchemeTask>)task {
    NSURLRequest *request = task.request;
    NSURLComponents *parts = [NSURLComponents componentsWithString:gSite];
    NSURLComponents *source = [NSURLComponents componentsWithURL:request.URL resolvingAgainstBaseURL:NO];
    parts.percentEncodedPath = [parts.percentEncodedPath stringByAppendingString:source.percentEncodedPath ?: @""];
    parts.percentEncodedQuery = source.percentEncodedQuery;
    NSURL *target = parts.URL;
    if (!target) {
        [self replyText:task status:502 text:@"bad site url"];
        return;
    }
    NSMutableURLRequest *out = [NSMutableURLRequest requestWithURL:target cachePolicy:NSURLRequestReloadIgnoringLocalCacheData timeoutInterval:30];
    out.HTTPMethod = request.HTTPMethod;
    for (NSString *key in request.allHTTPHeaderFields) {
        NSString *lower = key.lowercaseString;
        if ([lower isEqualToString:@"host"] || [lower isEqualToString:@"origin"] || [lower isEqualToString:@"referer"] ||
            [lower isEqualToString:@"content-length"])
            continue;
        [out setValue:request.allHTTPHeaderFields[key] forHTTPHeaderField:key];
    }
    if (request.HTTPBody) {
        out.HTTPBody = request.HTTPBody;
    } else if (request.HTTPBodyStream) {
        NSMutableData *body = [NSMutableData data];
        NSInputStream *stream = request.HTTPBodyStream;
        [stream open];
        uint8_t buffer[16384];
        NSInteger n;
        while ((n = [stream read:buffer maxLength:sizeof buffer]) > 0) [body appendBytes:buffer length:(NSUInteger)n];
        [stream close];
        out.HTTPBody = body;
    }
    __weak LRSchemeHandler *weakSelf = self;
    [[_proxy dataTaskWithRequest:out
               completionHandler:^(NSData *data, NSURLResponse *response, NSError *error) {
                   dispatch_async(dispatch_get_main_queue(), ^{
                       LRSchemeHandler *me = weakSelf;
                       if (!me) return;
                       if (error || ![response isKindOfClass:[NSHTTPURLResponse class]]) {
                           [me replyText:task status:502 text:[NSString stringWithFormat:@"the site did not answer: %@", error.localizedDescription]];
                           return;
                       }
                       NSHTTPURLResponse *http = (NSHTTPURLResponse *)response;
                       NSMutableDictionary *headers = [NSMutableDictionary dictionary];
                       for (NSString *key in http.allHeaderFields) {
                           NSString *lower = [key lowercaseString];
                           // NSURLSession already decoded the body: its encoding and length headers no longer describe it
                           if ([lower isEqualToString:@"content-encoding"] || [lower isEqualToString:@"content-length"] ||
                               [lower isEqualToString:@"transfer-encoding"] || [lower isEqualToString:@"connection"] ||
                               [lower isEqualToString:@"keep-alive"])
                               continue;
                           headers[key] = http.allHeaderFields[key];
                       }
                       headers[@"Content-Length"] = [NSString stringWithFormat:@"%lu", (unsigned long)data.length];
                       headers[@"Cache-Control"] = @"no-store";
                       [me reply:task status:http.statusCode headers:headers body:[request.HTTPMethod isEqualToString:@"HEAD"] ? nil : data];
                   });
               }] resume];
}

- (void)webView:(WKWebView *)webView stopURLSchemeTask:(id<WKURLSchemeTask>)task {
    [_active removeObject:task];
}

@end

// ---------------------------------------------------------------------------------------------------------------------
// Updater: asks the site, downloads a newer build into a stage folder, verifies it, installs it as a new folder
// ---------------------------------------------------------------------------------------------------------------------

typedef void (^LRUpdateDone)(NSString *outcome, NSString *detail);

@interface LRUpdater : NSObject
@property(nonatomic, copy) void (^onInstalled)(NSString *ident, NSString *kind);
- (void)checkWithCompletion:(LRUpdateDone)done;
@end

@implementation LRUpdater {
    dispatch_queue_t _queue;
    NSURLSession *_session;
    BOOL _running;
}

- (instancetype)init {
    if ((self = [super init])) {
        _queue = dispatch_queue_create("lr.updater", DISPATCH_QUEUE_SERIAL);
        NSURLSessionConfiguration *c = [NSURLSessionConfiguration ephemeralSessionConfiguration];
        c.timeoutIntervalForRequest = 20;
        c.timeoutIntervalForResource = 900;
        c.requestCachePolicy = NSURLRequestReloadIgnoringLocalAndRemoteCacheData;
        _session = [NSURLSession sessionWithConfiguration:c];
    }
    return self;
}

- (NSURL *)siteURL:(NSString *)name stamp:(BOOL)stamp {
    NSString *s = [NSString stringWithFormat:@"%@/%@", gSite, name];
    if (stamp) s = [s stringByAppendingFormat:@"?t=%lld", (long long)([[NSDate date] timeIntervalSince1970] * 1000)];
    return [NSURL URLWithString:s];
}

// A synchronous GET (this runs on the updater queue). `status` is 200 for non-HTTP (file://) sites.
- (NSData *)get:(NSURL *)url etag:(NSString *)etag status:(NSInteger *)status etagOut:(NSString **)etagOut error:(NSString **)error {
    NSMutableURLRequest *r = [NSMutableURLRequest requestWithURL:url cachePolicy:NSURLRequestReloadIgnoringLocalAndRemoteCacheData timeoutInterval:20];
    [r setValue:@"no-cache" forHTTPHeaderField:@"Cache-Control"];
    if (etag.length) [r setValue:etag forHTTPHeaderField:@"If-None-Match"];
    __block NSData *out = nil;
    __block NSInteger code = 0;
    __block NSString *tag = nil, *err = nil;
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    [[_session dataTaskWithRequest:r
                 completionHandler:^(NSData *data, NSURLResponse *response, NSError *e) {
                     if (e) {
                         err = e.localizedDescription;
                     } else if ([response isKindOfClass:[NSHTTPURLResponse class]]) {
                         code = [(NSHTTPURLResponse *)response statusCode];
                         tag = [(NSHTTPURLResponse *)response allHeaderFields][@"Etag"] ?: [(NSHTTPURLResponse *)response allHeaderFields][@"ETag"];
                         out = data;
                     } else {
                         code = 200;
                         out = data;
                     }
                     dispatch_semaphore_signal(done);
                 }] resume];
    dispatch_semaphore_wait(done, DISPATCH_TIME_FOREVER);
    if (status) *status = code;
    if (etagOut) *etagOut = tag;
    if (error) *error = err;
    return out;
}

// Streams one site file to `dest`; YES only for a 200 with a non-empty body.
- (BOOL)download:(NSString *)name to:(NSString *)dest etag:(NSString **)etagOut error:(NSString **)error {
    NSMutableURLRequest *r = [NSMutableURLRequest requestWithURL:[self siteURL:name stamp:YES]
                                                     cachePolicy:NSURLRequestReloadIgnoringLocalAndRemoteCacheData
                                                 timeoutInterval:60];
    __block BOOL ok = NO;
    __block NSString *err = nil, *tag = nil;
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    [[_session downloadTaskWithRequest:r
                     completionHandler:^(NSURL *location, NSURLResponse *response, NSError *e) {
                         if (e || !location) {
                             err = e.localizedDescription ?: @"no data";
                         } else if ([response isKindOfClass:[NSHTTPURLResponse class]] && [(NSHTTPURLResponse *)response statusCode] != 200) {
                             err = [NSString stringWithFormat:@"HTTP %ld", (long)[(NSHTTPURLResponse *)response statusCode]];
                         } else {
                             NSFileManager *fm = [NSFileManager defaultManager];
                             [fm removeItemAtPath:dest error:nil];
                             if ([fm moveItemAtURL:location toURL:[NSURL fileURLWithPath:dest] error:nil] && LRFileSize(dest) > 0) {
                                 ok = YES;
                                 if ([response isKindOfClass:[NSHTTPURLResponse class]]) {
                                     NSDictionary *h = [(NSHTTPURLResponse *)response allHeaderFields];
                                     tag = h[@"Etag"] ?: h[@"ETag"];
                                 }
                             } else {
                                 err = @"could not store or empty file";
                             }
                         }
                         dispatch_semaphore_signal(done);
                     }] resume];
    dispatch_semaphore_wait(done, DISPATCH_TIME_FOREVER);
    if (error) *error = err;
    if (etagOut) *etagOut = tag;
    return ok;
}

- (void)checkWithCompletion:(LRUpdateDone)done {
    @synchronized(self) {
        if (_running) {
            if (done) dispatch_async(dispatch_get_main_queue(), ^{ done(@"busy", @"a check is already running"); });
            return;
        }
        _running = YES;
    }
    dispatch_async(_queue, ^{
        NSString *detail = nil;
        NSString *outcome = [self run:&detail];
        @synchronized(self) {
            _running = NO;
        }
        LRLog(@"update: %@ (%@)", outcome, detail ?: @"");
        if (done) dispatch_async(dispatch_get_main_queue(), ^{ done(outcome, detail); });
    });
}

- (NSString *)run:(NSString **)detail {
    NSInteger status = 0;
    NSString *err = nil;
    NSData *raw = [self get:[self siteURL:@"version.json" stamp:YES] etag:nil status:&status etagOut:nil error:&err];
    if (!raw) {
        *detail = err ?: @"no answer";
        return @"offline";
    }
    id parsed = status == 200 ? [NSJSONSerialization JSONObjectWithData:raw options:0 error:nil] : nil;
    NSString *build = [parsed isKindOfClass:[NSDictionary class]] ? parsed[@"build"] : nil;
    if (!LRIsHex64(build)) {
        *detail = [NSString stringWithFormat:@"version.json: HTTP %ld, no usable build", (long)status];
        return @"error";
    }
    if ([gStore isBadBuild:build]) {
        *detail = [NSString stringWithFormat:@"build %@ failed to start here before; skipped", [build substringToIndex:12]];
        return @"skipped";
    }
    NSMutableArray *files = [NSMutableArray array];
    id listed = parsed[@"files"];  // optional: a deploy may name its files; the default list is used otherwise
    if ([listed isKindOfClass:[NSArray class]] && [listed count] > 0) {
        for (id n in listed) {
            if (LRSafeName(n) && ![n isEqualToString:@"version.json"] && ![n isEqualToString:@"meta.json"]) [files addObject:n];
        }
    }
    if (![files containsObject:@"index.html"] || ![files containsObject:@"last_route.wasm"]) files = [LRDefaultFiles() mutableCopy];

    NSString *latest = [gStore latestId];
    NSString *latestDir = [gStore dirForId:latest];
    if (!LRFolderComplete(latestDir)) latestDir = [gStore currentDir];
    NSString *haveBuild = LRVersionOf(latestDir)[@"build"];
    if ([haveBuild isEqualToString:build]) return [self refreshPages:latestDir files:files raw:raw detail:detail];
    return [self installBuild:build files:files raw:raw detail:detail];
}

- (NSString *)installBuild:(NSString *)build files:(NSArray *)files raw:(NSData *)raw detail:(NSString **)detail {
    NSFileManager *fm = [NSFileManager defaultManager];
    NSString *stage = [gStore.webDir stringByAppendingPathComponent:[NSString stringWithFormat:@".stage-%@", [[NSUUID UUID] UUIDString]]];
    [fm createDirectoryAtPath:stage withIntermediateDirectories:YES attributes:nil error:nil];
    NSDate *started = [NSDate date];
    NSMutableDictionary *etags = [NSMutableDictionary dictionary];
    for (NSString *name in files) {
        NSString *err = nil, *tag = nil;
        if (![self download:name to:[stage stringByAppendingPathComponent:name] etag:&tag error:&err]) {
            *detail = [NSString stringWithFormat:@"download of %@ failed: %@", name, err];
            [fm removeItemAtPath:stage error:nil];
            return @"error";
        }
        if (tag) etags[name] = tag;
    }
    NSString *wasm = [stage stringByAppendingPathComponent:@"last_route.wasm"];
    NSString *sha = LRSha256File(wasm);
    NSData *head = [[NSFileHandle fileHandleForReadingAtPath:wasm] readDataOfLength:4];
    static const unsigned char magic[4] = { 0x00, 'a', 's', 'm' };
    if (![sha isEqualToString:build] || head.length != 4 || memcmp(head.bytes, magic, 4) != 0) {
        *detail = [NSString stringWithFormat:@"last_route.wasm sha256 %@ does not match version.json %@", [sha substringToIndex:MIN((NSUInteger)12, sha.length)], [build substringToIndex:12]];
        [fm removeItemAtPath:stage error:nil];
        return @"error";
    }
    // a deploy that finished while we downloaded would have mixed files: version.json must still say the same
    NSInteger status = 0;
    NSData *again = [self get:[self siteURL:@"version.json" stamp:YES] etag:nil status:&status etagOut:nil error:nil];
    id parsed = again ? [NSJSONSerialization JSONObjectWithData:again options:0 error:nil] : nil;
    if (![parsed isKindOfClass:[NSDictionary class]] || ![parsed[@"build"] isEqualToString:build]) {
        *detail = @"the site changed while downloading; will try again next check";
        [fm removeItemAtPath:stage error:nil];
        return @"error";
    }
    [raw writeToFile:[stage stringByAppendingPathComponent:@"version.json"] atomically:YES];
    LRWriteJSON(@{ @"etags" : etags }, [stage stringByAppendingPathComponent:@"meta.json"]);
    NSString *ident = LRComputeId(stage);
    if (!ident || [gStore isBadId:ident] || ![gStore installStage:stage asId:ident]) {
        *detail = [NSString stringWithFormat:@"could not install %@", ident];
        [fm removeItemAtPath:stage error:nil];
        return @"error";
    }
    *detail = [NSString stringWithFormat:@"installed %@ (%lu files, %.1f s)", ident, (unsigned long)files.count, -[started timeIntervalSinceNow]];
    [self announce:ident kind:@"build"];
    return @"installed";
}

// Same wasm build: the small page files may still have changed (index.html, update.js ...). Ask for each with its etag.
- (NSString *)refreshPages:(NSString *)dir files:(NSArray *)files raw:(NSData *)raw detail:(NSString **)detail {
    NSFileManager *fm = [NSFileManager defaultManager];
    NSMutableDictionary *meta = [NSMutableDictionary dictionaryWithDictionary:LRReadJSON([dir stringByAppendingPathComponent:@"meta.json"]) ?: @{}];
    NSMutableDictionary *etags = [NSMutableDictionary dictionaryWithDictionary:meta[@"etags"] ?: @{}];
    NSMutableDictionary *changed = [NSMutableDictionary dictionary];
    for (NSString *name in files) {
        if ([name isEqualToString:@"last_route.wasm"]) continue;
        NSInteger status = 0;
        NSString *tag = nil, *err = nil;
        NSData *data = [self get:[self siteURL:name stamp:YES] etag:etags[name] status:&status etagOut:&tag error:&err];
        if (!data || status == 304) continue;
        if (status != 200 || data.length == 0) {
            *detail = [NSString stringWithFormat:@"%@: HTTP %ld", name, (long)status];
            return @"error";
        }
        NSString *local = LRSha256File([dir stringByAppendingPathComponent:name]);
        if ([local isEqualToString:LRSha256Data(data)]) {
            if (tag) etags[name] = tag;
            continue;
        }
        changed[name] = data;
        if (tag) etags[name] = tag;
    }
    if (changed.count == 0) {
        if (![etags isEqualToDictionary:meta[@"etags"] ?: @{}]) {
            meta[@"etags"] = etags;
            LRWriteJSON(meta, [dir stringByAppendingPathComponent:@"meta.json"]);
        }
        *detail = [NSString stringWithFormat:@"build %@ is current", [[LRVersionOf(dir)[@"build"] description] substringToIndex:12]];
        return @"uptodate";
    }
    NSString *stage = [gStore.webDir stringByAppendingPathComponent:[NSString stringWithFormat:@".stage-%@", [[NSUUID UUID] UUIDString]]];
    if (![fm copyItemAtPath:dir toPath:stage error:nil]) {
        *detail = @"could not copy the current build";
        return @"error";
    }
    for (NSString *name in changed) [changed[name] writeToFile:[stage stringByAppendingPathComponent:name] atomically:YES];
    [raw writeToFile:[stage stringByAppendingPathComponent:@"version.json"] atomically:YES];
    meta[@"etags"] = etags;
    LRWriteJSON(meta, [stage stringByAppendingPathComponent:@"meta.json"]);
    NSString *ident = LRComputeId(stage);
    if (!ident || [gStore isBadId:ident] || ![gStore installStage:stage asId:ident]) {
        [fm removeItemAtPath:stage error:nil];
        *detail = @"could not install the changed page files";
        return @"error";
    }
    *detail = [NSString stringWithFormat:@"installed %@ (page files changed: %@)", ident, [[changed allKeys] componentsJoinedByString:@", "]];
    [self announce:ident kind:@"pages"];
    return @"installed";
}

- (void)announce:(NSString *)ident kind:(NSString *)kind {
    void (^handler)(NSString *, NSString *) = self.onInstalled;
    dispatch_async(dispatch_get_main_queue(), ^{
        if (handler) handler(ident, kind);
    });
}

@end

// ---------------------------------------------------------------------------------------------------------------------
// The web view
// ---------------------------------------------------------------------------------------------------------------------

@interface LRWebView : WKWebView
@end

@implementation LRWebView
- (BOOL)acceptsFirstMouse:(NSEvent *)event {
    return YES;
}
// the system's right-click menu (Reload, Back, Inspect ...) has no place over a game
- (void)willOpenMenu:(NSMenu *)menu withEvent:(NSEvent *)event {
    [menu removeAllItems];
}
@end

API_AVAILABLE(macos(11.3))
@interface LRDownloadDelegate : NSObject <WKDownloadDelegate>
@end

@implementation LRDownloadDelegate {
    NSMutableDictionary *_targets;
}
- (void)download:(WKDownload *)download
    decideDestinationUsingResponse:(NSURLResponse *)response
                 suggestedFilename:(NSString *)suggestedFilename
                 completionHandler:(void (^)(NSURL *))completionHandler {
    NSURL *folder = [[NSFileManager defaultManager] URLsForDirectory:NSDownloadsDirectory inDomains:NSUserDomainMask].firstObject;
    NSString *name = LRSafeName(suggestedFilename) ? suggestedFilename : @"download";
    NSURL *target = [folder URLByAppendingPathComponent:name];
    int n = 1;
    while ([[NSFileManager defaultManager] fileExistsAtPath:target.path]) {
        target = [folder URLByAppendingPathComponent:[NSString stringWithFormat:@"%@ (%d).%@", name.stringByDeletingPathExtension, n++, name.pathExtension]];
    }
    if (!_targets) _targets = [NSMutableDictionary dictionary];
    _targets[[NSValue valueWithNonretainedObject:download]] = target;
    LRLog(@"download: %@ -> %@", name, target.path);
    completionHandler(target);
}
- (void)downloadDidFinish:(WKDownload *)download {
    NSURL *target = _targets[[NSValue valueWithNonretainedObject:download]];
    [_targets removeObjectForKey:[NSValue valueWithNonretainedObject:download]];
    LRLog(@"download: finished %@", target.path);
    if (target) [[NSWorkspace sharedWorkspace] activateFileViewerSelectingURLs:@[ target ]];
}
- (void)download:(WKDownload *)download didFailWithError:(NSError *)error resumeData:(NSData *)resumeData {
    [_targets removeObjectForKey:[NSValue valueWithNonretainedObject:download]];
    LRLog(@"download: failed: %@", error.localizedDescription);
}
@end

// ---------------------------------------------------------------------------------------------------------------------
// The application
// ---------------------------------------------------------------------------------------------------------------------

@interface LRApp : NSObject <NSApplicationDelegate, NSWindowDelegate, WKNavigationDelegate, WKUIDelegate, WKScriptMessageHandler>
@end

@implementation LRApp {
    NSWindow *_window;
    LRWebView *_web;
    LRSchemeHandler *_scheme;
    LRUpdater *_updater;
    NSTimer *_readyTimer;
    NSTimer *_checkTimer;
    BOOL _pageReady;
    BOOL _pageLoading;
    BOOL _failureHandled;
    BOOL _shown;
    BOOL _fullscreen;
    NSString *_bootId;
    NSString *_pendingActivation;
    NSDate *_launched;
    NSDate *_bootStarted;
    id _activity;
    id _keyMonitor;
    id _downloadDelegate;
    int _readyCount;
    BOOL _probeDone;
    NSString *_readyInfo;
}

// ---- start ----

- (void)applicationDidFinishLaunching:(NSNotification *)note {
    _launched = [NSDate date];
    struct utsname u;
    uname(&u);
    LRLog(@"==== Last Route web app %@ starting%@ on macOS %@ (%s)", APP_VERSION, gSelfTest ? @" [self-test]" : @"",
          [[NSProcessInfo processInfo] operatingSystemVersionString], u.machine);
    LRLog(@"site %@, data %@, ready timeout %.0f s, check every %.0f s%@", gSite, gDataDir, gReadyTimeout, gCheckInterval, gNoUpdate ? @" (updates off)" : @"");
    _activity = [[NSProcessInfo processInfo] beginActivityWithOptions:NSActivityUserInitiatedAllowingIdleSystemSleep reason:@"Last Route is running"];

    gStore = [LRStore new];
    NSDate *t0 = [NSDate date];
    [gStore setup];
    LRLog(@"store: current %@, previous %@, bundled %@ (%.0f ms)", [gStore currentId] ?: @"-", [gStore previousId] ?: @"-", gStore.bundleId ?: @"-", -[t0 timeIntervalSinceNow] * 1000);
    if (![gStore currentDir]) {
        [self fatal:@"This copy of Last Route has no game files and could not install any. Reinstall the app."];
        return;
    }
    // a page that was still starting when the app last died counts as a failed start of that build
    NSString *booting = [gStore bootingId];
    if (booting) {
        LRLog(@"store: the last run died while starting %@", booting);
        [self noteFailure:booting reason:@"the app ended while the page was starting"];
        [gStore setBooting:nil];
    }
    // the build that will run: integrity check of the wasm against version.json
    [self verifyCurrent];

    _updater = [LRUpdater new];
    __weak LRApp *weakSelf = self;
    _updater.onInstalled = ^(NSString *ident, NSString *kind) {
        [weakSelf installed:ident kind:kind];
    };
    if (gSelfTest && !gSelfTestVisible) [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
    [self buildMenu];
    [self buildWindow];
    [self installKeyMonitor];
    [self bootPage];
    if (!gNoUpdate) {
        _checkTimer = [NSTimer scheduledTimerWithTimeInterval:gCheckInterval target:self selector:@selector(periodicCheck:) userInfo:nil repeats:YES];
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(gFirstCheckDelay * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
            [weakSelf periodicCheck:nil];
        });
    }
}

- (void)fatal:(NSString *)text {
    LRLog(@"FATAL %@", text);
    if (gSelfTest) {
        [self finishSelfTest:2 text:text];
        return;
    }
    NSAlert *alert = [NSAlert new];
    alert.messageText = @"Last Route cannot start";
    alert.informativeText = text;
    [alert runModal];
    [NSApp terminate:nil];
}

- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)app {
    return YES;
}

- (void)applicationWillTerminate:(NSNotification *)note {
    // a normal quit during the start is not a failed start
    if (!_pageReady) [gStore setBooting:nil];
    LRLog(@"==== quit");
    LRLogFlush();
}

- (void)verifyCurrent {
    NSString *dir = [gStore currentDir];
    NSDate *t0 = [NSDate date];
    NSString *build = LRVersionOf(dir)[@"build"];
    NSString *sha = LRSha256File([dir stringByAppendingPathComponent:@"last_route.wasm"]);
    BOOL ok = [sha isEqualToString:build];
    LRLog(@"verify: %@ wasm sha256 %@ %@ (%.0f ms)", [gStore bootId], [sha substringToIndex:MIN((NSUInteger)12, sha.length)], ok ? @"matches version.json" : @"DOES NOT MATCH version.json", -[t0 timeIntervalSinceNow] * 1000);
    if (!ok) {
        NSString *ident = [gStore bootId];
        [gStore markBad:ident];
        NSString *target = [gStore rollbackFrom:ident];
        if (target) {
            LRLog(@"verify: damaged copy, running %@ instead", target);
        } else {
            LRLog(@"verify: damaged copy and nothing to fall back to");
        }
    }
}

// ---- window and web view ----

- (void)buildWindow {
    WKWebViewConfiguration *config = [WKWebViewConfiguration new];
    _scheme = [LRSchemeHandler new];
    [config setURLSchemeHandler:_scheme forURLScheme:kScheme];
    config.mediaTypesRequiringUserActionForPlayback = WKAudiovisualMediaTypeNone;
    config.preferences.javaScriptCanOpenWindowsAutomatically = NO;
    config.websiteDataStore = [WKWebsiteDataStore defaultDataStore];

    WKUserContentController *content = [WKUserContentController new];
    NSString *shell = [self resource:@"shell.js"];
    if (shell) {
        shell = [shell stringByReplacingOccurrencesOfString:@"@SITE@" withString:gSite];
        shell = [shell stringByReplacingOccurrencesOfString:@"@VERSION@" withString:APP_VERSION];
        [content addUserScript:[[WKUserScript alloc] initWithSource:shell injectionTime:WKUserScriptInjectionTimeAtDocumentStart forMainFrameOnly:YES]];
    } else {
        LRLog(@"WARNING shell.js is missing from the app; the game cannot reach its server and cannot report ready");
    }
    [content addScriptMessageHandler:self name:@"lr"];
    config.userContentController = content;

    NSRect frame = NSMakeRect(0, 0, 1280, 720);
    NSWindowStyleMask style = NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable;
    _window = [[NSWindow alloc] initWithContentRect:frame styleMask:style backing:NSBackingStoreBuffered defer:NO];
    _window.title = @"Last Route";
    _window.delegate = self;
    _window.backgroundColor = [NSColor blackColor];
    _window.releasedWhenClosed = NO;
    _window.minSize = NSMakeSize(640, 360);
    _window.collectionBehavior = NSWindowCollectionBehaviorFullScreenPrimary;
    _web = [[LRWebView alloc] initWithFrame:[_window.contentView bounds] configuration:config];
    _web.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    _web.navigationDelegate = self;
    _web.UIDelegate = self;
    _web.allowsBackForwardNavigationGestures = NO;
    _web.allowsMagnification = NO;
    [_window.contentView addSubview:_web];
    if (gSelfTest && !gSelfTestVisible) {
        _window.alphaValue = 0;
        _window.ignoresMouseEvents = YES;  // clicks aimed at whatever is behind it must pass through
        // on top of everything and on every desktop, so that a covering window cannot make the page think it is hidden and stop its frames
        _window.level = NSScreenSaverWindowLevel;
        _window.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorFullScreenAuxiliary | NSWindowCollectionBehaviorStationary;
        [_window setFrameOrigin:NSMakePoint(40, 40)];
    } else {
        [_window setFrameAutosaveName:@"LastRouteMain"];
        if (![_window setFrameUsingName:@"LastRouteMain"]) [_window center];
    }
}

- (NSString *)resource:(NSString *)name {
    NSString *path = [[[NSBundle mainBundle] resourcePath] stringByAppendingPathComponent:name];
    return [NSString stringWithContentsOfFile:path encoding:NSUTF8StringEncoding error:nil];
}

- (void)showWindow {
    if (_shown) return;
    _shown = YES;
    if (gSelfTest && !gSelfTestVisible) {
        [_window orderFrontRegardless];
        return;
    }
    [NSApp activateIgnoringOtherApps:YES];
    [_window makeKeyAndOrderFront:nil];
    [_window makeFirstResponder:_web];
}

// ---- booting the page and watching that it starts ----

// Starts judging a page load: it has gReadyTimeout seconds to report ready. Called for the loads the shell starts and for
// the ones the page starts itself (the game reloads itself after an update).
- (void)beginWatch {
    _bootId = [gStore bootId];
    _pageReady = NO;
    _pageLoading = YES;
    _failureHandled = NO;
    _bootStarted = [NSDate date];
    [gStore setBooting:_bootId];
    [_readyTimer invalidate];
    _readyTimer = [NSTimer scheduledTimerWithTimeInterval:gReadyTimeout target:self selector:@selector(readyTimeout:) userInfo:nil repeats:NO];
}

- (void)bootPage {
    [self beginWatch];
    _scheme.pinnedDir = nil;
    LRLog(@"boot: loading %@ (build folder %@)", kPageURL, _bootId);
    NSURLRequest *request = [NSURLRequest requestWithURL:[NSURL URLWithString:kPageURL] cachePolicy:NSURLRequestReloadIgnoringLocalAndRemoteCacheData timeoutInterval:30];
    [_web loadRequest:request];
    // the window opens at once; a page that is slow shows black, not a missing app
    [self showWindow];
}

- (void)readyTimeout:(NSTimer *)timer {
    [self bootFailed:[NSString stringWithFormat:@"the page did not report ready within %.0f s", gReadyTimeout]];
}

- (void)pageBecameReady:(NSDictionary *)info {
    if (_pageReady) return;
    _pageReady = YES;
    _pageLoading = NO;
    _readyCount += 1;
    [_readyTimer invalidate];
    _readyTimer = nil;
    [gStore clearFailure:_bootId];
    [gStore setBooting:nil];
    _readyInfo = [NSString stringWithFormat:@"%@", info];
    LRLog(@"boot: READY after %.1f s (build folder %@) %@", -[_bootStarted timeIntervalSinceNow], _bootId, info);
    [_window makeFirstResponder:_web];
    if (_pendingActivation) {
        NSString *ident = _pendingActivation;
        _pendingActivation = nil;
        [self activate:ident];
    }
    if (gSelfTest && _readyCount >= 2) {
        [self finishSelfTest:0 text:[NSString stringWithFormat:@"the page reloaded itself and started again on build folder %@", _bootId]];
        return;
    }
    if (gSelfTest) {
        // let the game run a moment, then probe the page
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC), dispatch_get_main_queue(), ^{
            NSString *probe = [self resource:@"selftest.js"];
            if (!probe) {
                [self finishSelfTest:1 text:@"selftest.js is missing from the app"];
                return;
            }
            LRLog(@"selftest: web view menu validation: paste=%d copy=%d cut=%d selectAll=%d", [self webViewCan:@selector(paste:)], [self webViewCan:@selector(copy:)], [self webViewCan:@selector(cut:)], [self webViewCan:@selector(selectAll:)]);
            [self->_web evaluateJavaScript:[probe stringByAppendingString:@"\n;undefined;"] completionHandler:^(id result, NSError *error) {
                if (error) LRLog(@"selftest: probe script error: %@", error.localizedDescription);
            }];
            dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 40 * NSEC_PER_SEC), dispatch_get_main_queue(), ^{
                if (self->_probeDone) return;
                [self finishSelfTest:1 text:@"the probe did not finish in 40 s"];
            });
        });
    }
}

// A failed start of the running page: retry once, then roll back (previous build, then the bundled one).
- (void)bootFailed:(NSString *)reason {
    if (_pageReady || _failureHandled) return;
    _failureHandled = YES;
    [_readyTimer invalidate];
    _readyTimer = nil;
    LRLog(@"boot: FAILED (%@) build folder %@", reason, _bootId);
    [self noteFailure:_bootId reason:reason];
    [gStore setBooting:nil];
    NSString *ident = _bootId;
    // noteFailure already rolled back when the limit was reached; either way the page is started again
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.8 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
        NSString *now = [gStore bootId];
        if ([now isEqualToString:ident] && [gStore isBadId:ident]) {
            [self giveUp:reason];
            return;
        }
        [self bootPage];
    });
}

// Counts a failed start of `ident`; at the limit the build is marked bad and the current pointer moves back.
- (void)noteFailure:(NSString *)ident reason:(NSString *)reason {
    int n = [gStore bumpFailure:ident];
    LRLog(@"boot: %@ has now failed %d time(s) in a row (limit %d)", ident, n, gMaxFailures);
    if (n < gMaxFailures) return;
    [gStore markBad:ident];
    NSString *target = [gStore rollbackFrom:ident];
    if (target) {
        LRLog(@"boot: ROLLED BACK from %@ to %@", ident, target);
    } else {
        LRLog(@"boot: no older build to roll back to from %@", ident);
    }
}

- (void)giveUp:(NSString *)reason {
    LRLog(@"boot: giving up: %@", reason);
    if (gSelfTest) {
        [self finishSelfTest:1 text:[NSString stringWithFormat:@"the game did not start: %@", reason]];
        return;
    }
    NSAlert *alert = [NSAlert new];
    alert.messageText = @"Last Route did not start";
    alert.informativeText = [NSString stringWithFormat:@"The game page failed to start (%@), and there is no older copy to go back to.\n\nYou can play in your web browser instead, try again, or reset the app to the build bundled inside it.", reason];
    [alert addButtonWithTitle:@"Open in Browser"];
    [alert addButtonWithTitle:@"Try Again"];
    [alert addButtonWithTitle:@"Reset to Bundled Build"];
    [alert addButtonWithTitle:@"Quit"];
    NSModalResponse response = [alert runModal];
    if (response == NSAlertFirstButtonReturn) {
        [[NSWorkspace sharedWorkspace] openURL:[NSURL URLWithString:gSite]];
        [NSApp terminate:nil];
    } else if (response == NSAlertSecondButtonReturn) {
        [gStore clearFailure:[gStore bootId]];
        [self bootPage];
    } else if (response == NSAlertThirdButtonReturn) {
        [gStore resetToBundled];
        [self bootPage];
    } else {
        [NSApp terminate:nil];
    }
}

// ---- updates ----

- (void)periodicCheck:(NSTimer *)timer {
    if (gNoUpdate) return;
    [_updater checkWithCompletion:nil];
}

- (void)checkForUpdates:(id)sender {
    if (gNoUpdate) {
        [self inform:@"Updates are turned off" text:@"The app was started with --no-update."];
        return;
    }
    [_updater checkWithCompletion:^(NSString *outcome, NSString *detail) {
        if ([outcome isEqualToString:@"uptodate"]) {
            [self inform:@"You have the latest version" text:@"The web files of this app match the website."];
        } else if ([outcome isEqualToString:@"installed"]) {
            [self inform:@"A new version was installed" text:@"The game offers to restart itself within a minute; choose View > Reload Game to switch now."];
        } else if ([outcome isEqualToString:@"offline"]) {
            [self inform:@"Could not reach the website" text:[NSString stringWithFormat:@"The game keeps running from the copy on this Mac.\n\n%@", detail]];
        } else {
            [self inform:@"The update check did not finish" text:detail ?: outcome];
        }
    }];
}

- (void)inform:(NSString *)title text:(NSString *)text {
    if (gSelfTest) return;
    NSAlert *alert = [NSAlert new];
    alert.messageText = title;
    alert.informativeText = text ?: @"";
    [alert beginSheetModalForWindow:_window completionHandler:nil];
}

- (void)installed:(NSString *)ident kind:(NSString *)kind {
    [gStore setPendingId:ident];
    // switching the pointer while the page is still loading could hand it a mix of two builds
    if (_pageLoading && !_pageReady) {
        LRLog(@"update: %@ is ready; switching once the page is up", ident);
        _pendingActivation = ident;
        return;
    }
    [self activate:ident];
}

- (void)activate:(NSString *)ident {
    [gStore activate:ident];
    LRLog(@"update: now current %@ (previous %@); a running page learns of it from version.json", ident, [gStore previousId] ?: @"-");
}

// ---- menus ----

- (NSMenuItem *)addItem:(NSString *)title action:(SEL)action key:(NSString *)key to:(NSMenu *)menu {
    NSMenuItem *item = [menu addItemWithTitle:title action:action keyEquivalent:key];
    return item;
}

- (void)buildMenu {
    NSMenu *bar = [NSMenu new];
    NSString *name = @"Last Route";

    NSMenuItem *appItem = [NSMenuItem new];
    [bar addItem:appItem];
    NSMenu *app = [[NSMenu alloc] initWithTitle:name];
    [self addItem:[@"About " stringByAppendingString:name] action:@selector(showAbout:) key:@"" to:app].target = self;
    [app addItem:[NSMenuItem separatorItem]];
    [self addItem:@"Check for Updates…" action:@selector(checkForUpdates:) key:@"" to:app].target = self;
    [self addItem:@"Reset to Bundled Build" action:@selector(resetBundled:) key:@"" to:app].target = self;
    [app addItem:[NSMenuItem separatorItem]];
    [self addItem:[@"Hide " stringByAppendingString:name] action:@selector(hide:) key:@"h" to:app];
    NSMenuItem *others = [self addItem:@"Hide Others" action:@selector(hideOtherApplications:) key:@"h" to:app];
    others.keyEquivalentModifierMask = NSEventModifierFlagCommand | NSEventModifierFlagOption;
    [self addItem:@"Show All" action:@selector(unhideAllApplications:) key:@"" to:app];
    [app addItem:[NSMenuItem separatorItem]];
    [self addItem:[@"Quit " stringByAppendingString:name] action:@selector(terminate:) key:@"q" to:app];
    appItem.submenu = app;

    NSMenuItem *editItem = [NSMenuItem new];
    [bar addItem:editItem];
    NSMenu *edit = [[NSMenu alloc] initWithTitle:@"Edit"];
    [self addItem:@"Cut" action:@selector(cut:) key:@"x" to:edit];
    [self addItem:@"Copy" action:@selector(copy:) key:@"c" to:edit];
    [self addItem:@"Paste" action:@selector(paste:) key:@"v" to:edit];
    [self addItem:@"Select All" action:@selector(selectAll:) key:@"a" to:edit];
    editItem.submenu = edit;

    NSMenuItem *viewItem = [NSMenuItem new];
    [bar addItem:viewItem];
    NSMenu *view = [[NSMenu alloc] initWithTitle:@"View"];
    [self addItem:@"Reload Game" action:@selector(reloadGame:) key:@"r" to:view].target = self;
    NSMenuItem *fs = [self addItem:@"Enter Full Screen" action:@selector(toggleFullScreen:) key:@"f" to:view];
    fs.keyEquivalentModifierMask = NSEventModifierFlagCommand | NSEventModifierFlagControl;
    [view addItem:[NSMenuItem separatorItem]];
    [self addItem:@"Open Game Website in Browser" action:@selector(openSite:) key:@"" to:view].target = self;
    [self addItem:@"Show Log in Finder" action:@selector(showLog:) key:@"" to:view].target = self;
    viewItem.submenu = view;

    NSMenuItem *windowItem = [NSMenuItem new];
    [bar addItem:windowItem];
    NSMenu *window = [[NSMenu alloc] initWithTitle:@"Window"];
    [self addItem:@"Minimize" action:@selector(performMiniaturize:) key:@"m" to:window];
    [self addItem:@"Zoom" action:@selector(performZoom:) key:@"" to:window];
    [window addItem:[NSMenuItem separatorItem]];
    [self addItem:@"Bring All to Front" action:@selector(arrangeInFront:) key:@"" to:window];
    windowItem.submenu = window;

    [NSApp setMainMenu:bar];
    [NSApp setWindowsMenu:window];
}

- (void)showAbout:(id)sender {
    NSString *build = LRVersionOf([gStore currentDir])[@"build"];
    NSString *built = LRVersionOf([gStore currentDir])[@"built"];
    NSString *credits = [NSString stringWithFormat:@"Web build %@\nbuilt %@\nsite %@\nlog %@",
                         [build isKindOfClass:[NSString class]] ? [build substringToIndex:MIN((NSUInteger)12, build.length)] : @"?",
                         built ?: @"?", gSite, gLogPath];
    NSDictionary *options = @{
        @"ApplicationName" : @"Last Route",
        @"ApplicationVersion" : APP_VERSION,
        @"Version" : [gStore bootId] ?: @"",
        @"Credits" : [[NSAttributedString alloc] initWithString:credits],
    };
    [NSApp activateIgnoringOtherApps:YES];
    [NSApp orderFrontStandardAboutPanelWithOptions:options];
}

- (void)reloadGame:(id)sender {
    LRLog(@"menu: reload");
    [self bootPage];
}

- (void)resetBundled:(id)sender {
    if (!gStore.bundleDir) {
        [self inform:@"No bundled build" text:@"This copy of the app does not carry web files of its own."];
        return;
    }
    LRLog(@"menu: reset to the bundled build");
    [gStore resetToBundled];
    [self bootPage];
}

- (void)openSite:(id)sender {
    [[NSWorkspace sharedWorkspace] openURL:[NSURL URLWithString:gSite]];
}

- (void)showLog:(id)sender {
    [[NSWorkspace sharedWorkspace] activateFileViewerSelectingURLs:@[ [NSURL fileURLWithPath:gLogPath] ]];
}

// ---- keys: the menu shortcuts must work even when the game page takes the key press first ----

- (void)installKeyMonitor {
    __weak LRApp *weakSelf = self;
    _keyMonitor = [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskKeyDown
                                                        handler:^NSEvent *(NSEvent *event) {
                                                            return [weakSelf filterKey:event];
                                                        }];
}

- (NSEvent *)filterKey:(NSEvent *)event {
    NSEventModifierFlags flags = event.modifierFlags & NSEventModifierFlagDeviceIndependentFlagsMask;
    NSString *chars = event.charactersIgnoringModifiers.lowercaseString;
    if (!(flags & NSEventModifierFlagCommand) || chars.length != 1) return event;
    BOOL plain = (flags & ~(NSEventModifierFlagCommand | NSEventModifierFlagCapsLock | NSEventModifierFlagNumericPad | NSEventModifierFlagFunction)) == 0;
    unichar c = [chars characterAtIndex:0];
    if (plain && (c == 'q' || c == 'r' || c == 'm' || c == 'h')) {
        if ([NSApp.mainMenu performKeyEquivalent:event]) return nil;
    }
    if (flags & NSEventModifierFlagControl && c == 'f') {
        if ([NSApp.mainMenu performKeyEquivalent:event]) return nil;
    }
    // Cmd+V: a game page has no editable element, so WKWebView may not turn the key into a paste event. When the web view says
    // paste is not available, the shell reads the clipboard text itself (the user just asked for it) and hands it to the page.
    if (plain && c == 'v' && _window.firstResponder == _web && ![self webViewCanPaste]) {
        NSString *text = [[NSPasteboard generalPasteboard] stringForType:NSPasteboardTypeString];
        if (text.length > 0) {
            [self deliverPaste:text];
            return nil;
        }
    }
    return event;
}

- (BOOL)webViewCan:(SEL)action {
    NSMenuItem *probe = [[NSMenuItem alloc] initWithTitle:@"x" action:action keyEquivalent:@""];
    if (![_web respondsToSelector:@selector(validateUserInterfaceItem:)]) return NO;
    return [(id)_web validateUserInterfaceItem:probe];
}

- (BOOL)webViewCanPaste {
    return [self webViewCan:@selector(paste:)];
}

- (void)deliverPaste:(NSString *)text {
    if (text.length > 4096) text = [text substringToIndex:4096];
    NSData *json = [NSJSONSerialization dataWithJSONObject:@[ text ] options:0 error:nil];
    NSString *array = [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
    NSString *script = [NSString stringWithFormat:@"window.__lrPaste(%@[0])", array];
    [_web evaluateJavaScript:script completionHandler:^(id result, NSError *error) {
        LRLog(@"paste: handed %lu characters to the page, result %@%@", (unsigned long)text.length, result, error ? error.localizedDescription : @"");
    }];
}

// ---- window delegate: fullscreen state reaches the page ----

- (void)windowDidEnterFullScreen:(NSNotification *)n {
    _fullscreen = YES;
    [_web evaluateJavaScript:@"window.__lrFullscreen && window.__lrFullscreen(true)" completionHandler:nil];
}

- (void)windowDidExitFullScreen:(NSNotification *)n {
    _fullscreen = NO;
    [_web evaluateJavaScript:@"window.__lrFullscreen && window.__lrFullscreen(false)" completionHandler:nil];
}

- (void)windowDidBecomeKey:(NSNotification *)n {
    [_window makeFirstResponder:_web];
    [_web evaluateJavaScript:@"(function(){var c=document.getElementById('glcanvas'); if(c) c.focus();})()" completionHandler:nil];
}

// ---- messages from the page ----

- (void)userContentController:(WKUserContentController *)controller didReceiveScriptMessage:(WKScriptMessage *)message {
    NSDictionary *m = [message.body isKindOfClass:[NSDictionary class]] ? message.body : nil;
    NSString *type = m[@"type"];
    if ([type isEqualToString:@"ready"]) {
        [self pageBecameReady:m[@"info"]];
    } else if ([type isEqualToString:@"error"]) {
        LRLog(@"page error: %@", m[@"text"]);
        if ([m[@"fatal"] boolValue]) [self bootFailed:[NSString stringWithFormat:@"page error: %@", m[@"text"]]];
    } else if ([type isEqualToString:@"console"]) {
        LRLog(@"page %@: %@", m[@"level"], m[@"text"]);
    } else if ([type isEqualToString:@"fullscreen"]) {
        BOOL want = [m[@"on"] boolValue];
        if (want != _fullscreen && !gSelfTest) [_window toggleFullScreen:nil];
    } else if ([type isEqualToString:@"selftest-click"]) {
        [self selfTestClick];
    } else if ([type isEqualToString:@"selftest-move"]) {
        [self selfTestMove];
    } else if ([type isEqualToString:@"selftest"]) {
        [self selfTestResult:m[@"result"]];
    }
}

// ---- navigation ----

- (void)webView:(WKWebView *)webView decidePolicyForNavigationAction:(WKNavigationAction *)action decisionHandler:(void (^)(WKNavigationActionPolicy))decisionHandler {
    NSURL *url = action.request.URL;
    NSString *scheme = url.scheme.lowercaseString;
    if (@available(macOS 11.3, *)) {
        if (action.shouldPerformDownload) {
            decisionHandler(WKNavigationActionPolicyDownload);
            return;
        }
    }
    if ([scheme isEqualToString:kScheme] || [scheme isEqualToString:@"about"] || [scheme isEqualToString:@"blob"] || [scheme isEqualToString:@"data"]) {
        decisionHandler(WKNavigationActionPolicyAllow);
        return;
    }
    if ([scheme isEqualToString:@"http"] || [scheme isEqualToString:@"https"] || [scheme isEqualToString:@"mailto"]) {
        LRLog(@"navigation: opening %@ in the default browser", url.absoluteString);
        [[NSWorkspace sharedWorkspace] openURL:url];
    }
    decisionHandler(WKNavigationActionPolicyCancel);
}

- (void)webView:(WKWebView *)webView decidePolicyForNavigationResponse:(WKNavigationResponse *)response decisionHandler:(void (^)(WKNavigationResponsePolicy))decisionHandler {
    if (@available(macOS 11.3, *)) {
        if (!response.canShowMIMEType) {
            decisionHandler(WKNavigationResponsePolicyDownload);
            return;
        }
    }
    decisionHandler(WKNavigationResponsePolicyAllow);
}

- (void)webView:(WKWebView *)webView navigationAction:(WKNavigationAction *)action didBecomeDownload:(WKDownload *)download API_AVAILABLE(macos(11.3)) {
    if (!_downloadDelegate) _downloadDelegate = [LRDownloadDelegate new];
    download.delegate = _downloadDelegate;
}

- (void)webView:(WKWebView *)webView navigationResponse:(WKNavigationResponse *)response didBecomeDownload:(WKDownload *)download API_AVAILABLE(macos(11.3)) {
    if (!_downloadDelegate) _downloadDelegate = [LRDownloadDelegate new];
    download.delegate = _downloadDelegate;
}

- (void)webView:(WKWebView *)webView didStartProvisionalNavigation:(WKNavigation *)navigation {
    if (_pageReady) {
        LRLog(@"navigation: the page is loading again by itself (an update reload)");
        [self beginWatch];
    }
}

- (void)webView:(WKWebView *)webView didFailProvisionalNavigation:(WKNavigation *)navigation withError:(NSError *)error {
    LRLog(@"navigation: failed to load: %@", error.localizedDescription);
    [self bootFailed:[NSString stringWithFormat:@"the page could not be loaded: %@", error.localizedDescription]];
}

- (void)webView:(WKWebView *)webView didFailNavigation:(WKNavigation *)navigation withError:(NSError *)error {
    LRLog(@"navigation: failed: %@", error.localizedDescription);
    [self bootFailed:[NSString stringWithFormat:@"the page failed: %@", error.localizedDescription]];
}

- (void)webViewWebContentProcessDidTerminate:(WKWebView *)webView {
    LRLog(@"navigation: the web content process ended");
    if (_pageReady) {
        // the game crashed after it had started: load it again (a build that crashes this way gets no rollback by itself)
        [self bootPage];
    } else {
        [self bootFailed:@"the web content process ended while the page was starting"];
    }
}

// ---- UI delegate: new windows, dialogs, the file chooser ----

- (WKWebView *)webView:(WKWebView *)webView createWebViewWithConfiguration:(WKWebViewConfiguration *)configuration forNavigationAction:(WKNavigationAction *)action windowFeatures:(WKWindowFeatures *)features {
    NSURL *url = action.request.URL;
    if ([url.scheme isEqualToString:@"http"] || [url.scheme isEqualToString:@"https"]) [[NSWorkspace sharedWorkspace] openURL:url];
    return nil;
}

- (void)webView:(WKWebView *)webView runJavaScriptAlertPanelWithMessage:(NSString *)message initiatedByFrame:(WKFrameInfo *)frame completionHandler:(void (^)(void))completionHandler {
    NSAlert *alert = [NSAlert new];
    alert.messageText = message ?: @"";
    [alert runModal];
    completionHandler();
}

- (void)webView:(WKWebView *)webView runJavaScriptConfirmPanelWithMessage:(NSString *)message initiatedByFrame:(WKFrameInfo *)frame completionHandler:(void (^)(BOOL))completionHandler {
    NSAlert *alert = [NSAlert new];
    alert.messageText = message ?: @"";
    [alert addButtonWithTitle:@"OK"];
    [alert addButtonWithTitle:@"Cancel"];
    completionHandler([alert runModal] == NSAlertFirstButtonReturn);
}

- (void)webView:(WKWebView *)webView runOpenPanelWithParameters:(WKOpenPanelParameters *)parameters initiatedByFrame:(WKFrameInfo *)frame completionHandler:(void (^)(NSArray<NSURL *> *))completionHandler {
    NSOpenPanel *panel = [NSOpenPanel openPanel];
    panel.allowsMultipleSelection = parameters.allowsMultipleSelection;
    BOOL directories = NO;
    if ([parameters respondsToSelector:@selector(allowsDirectories)]) directories = [(id)parameters allowsDirectories];
    panel.canChooseDirectories = directories;
    panel.canChooseFiles = !directories;
    [panel beginSheetModalForWindow:_window completionHandler:^(NSModalResponse result) {
        completionHandler(result == NSModalResponseOK ? panel.URLs : nil);
    }];
}

// ---- pointer lock ----
//
// WKWebView has no public API for the Pointer Lock API: without an answer from its UI delegate WebKit refuses every
// requestPointerLock() (pointerlockerror). WebKit asks the delegate through these two methods of its private WKUIDelegate
// interface (looked up with respondsToSelector, so a WebKit that does not know them just never calls them and the game keeps
// its "click to look around" fallback). They only say yes; WebKit does the rest (hides the cursor, sends movementX/Y).

- (void)_webViewDidRequestPointerLock:(WKWebView *)webView completionHandler:(void (^)(BOOL))completionHandler {
    LRLog(@"pointer lock: requested by the page, allowed");
    completionHandler(YES);
}

- (void)_webViewDidLosePointerLock:(WKWebView *)webView {
    LRLog(@"pointer lock: released");
}

// ---- self test ----

- (void)selfTestClick {
    // a real mouse press delivered to the web view: it is the user gesture a pointer-lock request needs
    NSPoint inView = NSMakePoint(NSWidth(_web.bounds) * 0.5, NSHeight(_web.bounds) * 0.1);
    NSPoint p = [_web convertPoint:inView toView:nil];
    NSTimeInterval t = [[NSProcessInfo processInfo] systemUptime];
    NSEvent *down = [NSEvent mouseEventWithType:NSEventTypeLeftMouseDown location:p modifierFlags:0 timestamp:t windowNumber:_window.windowNumber context:nil eventNumber:1 clickCount:1 pressure:1];
    NSEvent *up = [NSEvent mouseEventWithType:NSEventTypeLeftMouseUp location:p modifierFlags:0 timestamp:t + 0.05 windowNumber:_window.windowNumber context:nil eventNumber:1 clickCount:1 pressure:0];
    [_window makeKeyWindow];
    [_window sendEvent:down];
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.08 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
        [self->_window sendEvent:up];
    });
}

- (void)selfTestMove {
    NSPoint inView = NSMakePoint(NSWidth(_web.bounds) * 0.5, NSHeight(_web.bounds) * 0.5);
    NSPoint p = [_web convertPoint:inView toView:nil];
    NSPoint screen = [_window convertRectToScreen:NSMakeRect(p.x, p.y, 0, 0)].origin;
    CGFloat top = NSMaxY([[NSScreen screens] firstObject].frame);
    CGEventRef cg = CGEventCreateMouseEvent(NULL, kCGEventMouseMoved, CGPointMake(screen.x, top - screen.y), kCGMouseButtonLeft);
    CGEventSetIntegerValueField(cg, kCGMouseEventDeltaX, 17);
    CGEventSetIntegerValueField(cg, kCGMouseEventDeltaY, -5);
    NSEvent *event = [NSEvent eventWithCGEvent:cg];
    CFRelease(cg);
    LRLog(@"selftest: sending a mouse-moved event with delta (%.0f, %.0f) to the window", event.deltaX, event.deltaY);
    [_window sendEvent:event];
}

- (void)selfTestResult:(NSDictionary *)result {
    BOOL pass = YES;
    NSMutableArray *notes = [NSMutableArray array];
    NSArray *keys = [[result allKeys] sortedArrayUsingSelector:@selector(compare:)];
    for (NSString *key in keys) {
        NSData *json = [NSJSONSerialization dataWithJSONObject:@{ key : result[key] } options:0 error:nil];
        NSString *line = json ? [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding] : [NSString stringWithFormat:@"%@", result[key]];
        LRLog(@"selftest: %@", line);
    }
    NSDictionary *wasm = result[@"wasm"];
    NSDictionary *gl = result[@"webgl"];
    NSDictionary *vj = result[@"versionJson"];
    if (![wasm isKindOfClass:[NSDictionary class]] || [wasm[@"exports"] intValue] == 0) { pass = NO; [notes addObject:@"wasm not instantiated"]; }
    if (![gl isKindOfClass:[NSDictionary class]] || ![gl[@"gameContext"] boolValue]) { pass = NO; [notes addObject:@"no WebGL context"]; }
    if (![vj isKindOfClass:[NSDictionary class]] || [vj[@"status"] intValue] != 200) { pass = NO; [notes addObject:@"version.json not served"]; }
    _probeDone = YES;
    if (pass && gSelfTestHold > 0) {
        LRLog(@"selftest: probes passed; holding up to %.0f s to watch for an update reload", gSelfTestHold);
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(gSelfTestHold * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
            [self finishSelfTest:0 text:@"probes passed; hold time over"];
        });
        return;
    }
    [self finishSelfTest:pass ? 0 : 1 text:pass ? @"all required probes passed" : [notes componentsJoinedByString:@", "]];
}

- (void)finishSelfTest:(int)code text:(NSString *)text {
    static BOOL done = NO;
    if (done) return;
    done = YES;
    LRLog(@"selftest: %@ - %@ (ran %.1f s, build folder %@, ready %@)", code == 0 ? @"PASS" : @"FAIL", text, -[_launched timeIntervalSinceNow], [gStore bootId], _pageReady ? @"yes" : @"no");
    LRLogFlush();
    exit(code);
}

@end

// ---------------------------------------------------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------------------------------------------------

static void LRUsage(void) {
    printf("Last Route (web app shell) %s\n"
           "  --site-url URL        website to take the web files and the API from (default %s)\n"
           "  --data-dir DIR        where the downloaded web files live (default ~/Library/Application Support/LastRoute)\n"
           "  --log FILE            log file (default ~/Library/Logs/LastRoute/app.log)\n"
           "  --no-update           never ask the website (offline mode)\n"
           "  --check-interval S    seconds between update checks (default 600)\n"
           "  --ready-timeout S     seconds the page gets to start (default 90)\n"
           "  --max-failures N      failed starts in a row before a rollback (default 2)\n"
           "  --self-test           start without a visible window, probe the page, log the result and exit (0 = pass)\n"
           "  --version, --help\n"
           "Environment: LASTROUTE_SITE_URL, LASTROUTE_DATA_DIR.\n",
           APP_VERSION.UTF8String, kDefaultSite.UTF8String);
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        NSString *home = NSHomeDirectory();
        NSDictionary *env = [[NSProcessInfo processInfo] environment];
        gSite = env[@"LASTROUTE_SITE_URL"] ?: kDefaultSite;
        gDataDir = env[@"LASTROUTE_DATA_DIR"] ?: [home stringByAppendingPathComponent:@"Library/Application Support/LastRoute"];
        gLogPath = [home stringByAppendingPathComponent:@"Library/Logs/LastRoute/app.log"];
        for (int i = 1; i < argc; i++) {
            NSString *a = [NSString stringWithUTF8String:argv[i]];
            NSString *next = i + 1 < argc ? [NSString stringWithUTF8String:argv[i + 1]] : nil;
            if ([a isEqualToString:@"--help"] || [a isEqualToString:@"-h"]) {
                LRUsage();
                return 0;
            } else if ([a isEqualToString:@"--version"]) {
                printf("%s\n", APP_VERSION.UTF8String);
                return 0;
            } else if ([a isEqualToString:@"--self-test"]) {
                gSelfTest = YES;
            } else if ([a isEqualToString:@"--self-test-visible"]) {
                gSelfTest = YES;
                gSelfTestVisible = YES;
            } else if ([a isEqualToString:@"--no-update"]) {
                gNoUpdate = YES;
            } else if ([a isEqualToString:@"--site-url"] && next) {
                gSite = next, i++;
            } else if ([a isEqualToString:@"--data-dir"] && next) {
                gDataDir = [next stringByExpandingTildeInPath], i++;
            } else if ([a isEqualToString:@"--log"] && next) {
                gLogPath = [next stringByExpandingTildeInPath], i++;
            } else if ([a isEqualToString:@"--check-interval"] && next) {
                gCheckInterval = MAX(2, next.doubleValue), i++;
            } else if ([a isEqualToString:@"--self-test-hold"] && next) {
                gSelfTestHold = MAX(0, next.doubleValue), i++;
            } else if ([a isEqualToString:@"--first-check-delay"] && next) {
                gFirstCheckDelay = MAX(0, next.doubleValue), i++;
            } else if ([a isEqualToString:@"--ready-timeout"] && next) {
                gReadyTimeout = MAX(2, next.doubleValue), i++;
            } else if ([a isEqualToString:@"--max-failures"] && next) {
                gMaxFailures = MAX(1, next.intValue), i++;
            }
        }
        while ([gSite hasSuffix:@"/"]) gSite = [gSite substringToIndex:gSite.length - 1];
        gLogEcho = gSelfTest;
        LRLogOpen();
        NSApplication *app = [NSApplication sharedApplication];
        LRApp *delegate = [LRApp new];
        app.delegate = delegate;
        [app run];
    }
    return 0;
}
