/**
 * checks.js
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * What the front end has to keep doing.
 *
 * Each check writes one line into `#out`, ending in `yes` or `no`, which is
 * what `tools/ui_harness.py` reads back out of the rendered page. Every one of
 * them is here because the thing it checks broke once:
 *
 * * the grid drew nothing because a renderer threw on a field the interface
 *   had stopped asking for, and an exception in the render loop leaves every
 *   row after it blank;
 * * a poll loop that skipped its sort stopped repainting, because the sort is
 *   what refreshes a buffered view;
 * * a constant was put on an object that does not exist, which is a blank page
 *   at load time and nothing in the console anybody reads.
 */
(function () {
    var log = [];

    function say(what, ok) {
        log.push(what + ': ' + (ok ? 'yes' : 'no'));
        document.getElementById('out').textContent = log.join('\n');
    }

    /**
     * Hands the report back to whatever started the browser.
     *
     * The page is readable on its own — that is what `#out` is for — and this
     * is what lets a run end when the checks do rather than when a browser
     * decides the page has settled.
     */
    function report() {
        try {
            var request = new XMLHttpRequest();
            request.open('POST', '/results', true);
            request.send(log.join('\n'));
        } catch (error) {
            // Opened by hand rather than by the runner: the page is the report.
        }
    }

    function note(what) {
        log.push(what);
        document.getElementById('out').textContent = log.join('\n');
    }

    /** A torrent as the daemon reports one, with every key the grid knows. */
    function torrents(count) {
        var out = {};
        for (var i = 0; i < count; i++) {
            var id = ('0000000000000000000000000000000000000000' + i).slice(-40);
            out[id] = {
                queue: i,
                name: 'Harness.Item.' + i + '.1080p.WEB-DL.x265-TEST',
                total_wanted: 4294967296,
                state: 'Seeding',
                progress: 100.0,
                num_seeds: 4,
                total_seeds: 40,
                num_peers: 2,
                total_peers: 9,
                download_payload_rate: 0,
                upload_payload_rate: 24000,
                eta: 0,
                ratio: 1.4,
                distributed_copies: 2.1,
                is_auto_managed: true,
                time_added: 1700000000,
                tracker_host: 'tracker.example.org',
                download_location: '/downloads',
                last_seen_complete: 1700000000,
                total_done: 4294967296,
                total_uploaded: 812000000,
                max_download_speed: -1,
                max_upload_speed: -1,
                seeds_peers_ratio: 4.4,
                total_remaining: 0,
                completed_time: 1700000000,
                time_since_transfer: 4000,
                label: 'series',
                owner: 'localclient',
                idle_since: 0,
                idle_pause_at: 0,
                idle_resume_at: 0,
                space_paused: false,
                tracker_move_at: 0,
                tracker_remove_at: 0,
            };
        }
        return out;
    }

    Ext.onReady(function () {
        var grid;
        try {
            grid = new Deluge.TorrentGrid({ renderTo: 'grid', height: 420 });
            deluge.torrents = grid;
        } catch (error) {
            say('the torrent grid is built without throwing (' + error.message + ')', false);
            report();
            return;
        }
        say('the torrent grid is built', true);

        // --- the list arrives and is drawn ------------------------------
        var all = torrents(25);
        grid.update(all, true);
        say('every torrent reaches the store', grid.getStore().getCount() === 25);
        say('and every one of them is drawn', grid.getView().getRows().length === 25);

        // --- only the keys the visible columns need ---------------------
        var keys = Deluge.Keys.forGrid(grid);
        say(
            'fewer keys are asked for than the grid knows (' + keys.length + ' of ' + Deluge.Keys.Grid.length + ')',
            keys.length > 0 && keys.length < Deluge.Keys.Grid.length
        );
        say(
            'the keys nothing draws but everything needs are asked for',
            ['state', 'label', 'queue', 'idle_resume_at', 'space_paused'].every(function (key) {
                return keys.indexOf(key) !== -1;
            })
        );

        var model = grid.getColumnModel();
        var seeds = model.findColumnIndex('num_seeds');
        model.setHidden(seeds, false);
        var shown = Deluge.Keys.forGrid(grid);
        say(
            'showing a column asks for its field and the one its renderer reads',
            shown.indexOf('num_seeds') !== -1 && shown.indexOf('total_seeds') !== -1
        );
        model.setHidden(seeds, true);
        say(
            'hiding it stops asking for either',
            Deluge.Keys.forGrid(grid).indexOf('num_seeds') === -1
        );

        grid.getStore().sort('time_added', 'DESC');
        var added = model.findColumnIndex('time_added');
        var wasHidden = model.isHidden(added);
        model.setHidden(added, true);
        say(
            'a hidden column that is sorted on keeps its field',
            Deluge.Keys.forGrid(grid).indexOf('time_added') !== -1
        );
        model.setHidden(added, wasHidden);

        // --- a row holding only those keys still draws ------------------
        // ExtJS renders hidden columns too, into a style that hides them, so
        // every renderer runs on every row — including the ones whose field
        // is no longer asked for. One that throws takes the grid with it.
        var narrowed = {};
        var first = Object.keys(all)[0];
        narrowed[first] = {};
        Deluge.Keys.forGrid(grid).forEach(function (key) {
            if (key in all[first]) narrowed[first][key] = all[first][key];
        });
        grid.update(narrowed, true);
        var drew = true;
        try {
            grid.getView().refresh();
        } catch (error) {
            drew = false;
            note('  the renderer that threw: ' + error.message);
        }
        say('a row with only the asked-for fields draws', drew);
        say('and the grid shows it', grid.getView().getRows().length === 1);

        // --- differences are folded into the list -----------------------
        grid.update(torrents(4), true);
        var ids = Object.keys(grid.lastTorrents);
        var change = {};
        change[ids[0]] = { upload_payload_rate: 999000 };
        var merged = grid.merge(change, [ids[3]]);
        say(
            'a difference is applied and the rest is kept',
            Object.keys(merged).length === 3 &&
                merged[ids[0]].upload_payload_rate === 999000 &&
                merged[ids[0]].name === grid.lastTorrents[ids[0]].name &&
                !merged[ids[3]]
        );

        var fresh = torrents(1);
        var brought = {};
        brought['ffffffffffffffffffffffffffffffffffffffff'] = fresh[Object.keys(fresh)[0]];
        var after = grid.merge(brought, []);
        say(
            'a torrent the browser has never seen arrives whole',
            !!after['ffffffffffffffffffffffffffffffffffffffff']
        );
        grid.update(after, true);
        say('and the grid takes the merged list', grid.getStore().getCount() === 4);

        // --- a changed value repaints -----------------------------------
        // The sort at the end of `update` is what refreshes a buffered view.
        // Skipping it looks like an easy saving and freezes the grid instead.
        grid.update(torrents(3), true);
        grid.getStore().sort('name', 'ASC');
        var row = function () {
            var node = grid.getView().getRow(0);
            return node ? node.textContent : '';
        };
        var before = row();
        var moved = torrents(3);
        Object.keys(moved).forEach(function (id) {
            moved[id].upload_payload_rate = 999000;
        });
        grid.update(moved);
        var now = row();
        say('a changed value repaints the row', now !== before && now.indexOf('975.6') !== -1);

        // --- the poll loop ----------------------------------------------
        var polls = 0;
        var realUpdate = deluge.ui.update;
        deluge.ui.update = function () {
            polls++;
        };
        var visible = true;
        Object.defineProperty(document, 'hidden', {
            configurable: true,
            get: function () {
                return !visible;
            },
        });

        deluge.ui.running = undefined;
        visible = false;
        deluge.ui.schedule();
        say('a hidden tab schedules no poll', deluge.ui.running === undefined);

        visible = true;
        deluge.ui.schedule();
        say('a visible tab schedules one', deluge.ui.running !== undefined);
        clearTimeout(deluge.ui.running);
        deluge.ui.running = undefined;

        visible = false;
        deluge.ui.running = setTimeout(function () {}, 5000);
        deluge.ui.onVisibilityChanged();
        say('hiding it drops the poll already waiting', deluge.ui.running === undefined);
        say('and arms the call that keeps the session alive', deluge.ui.heartbeat !== undefined);

        var askedBefore = polls;
        visible = true;
        deluge.ui.onVisibilityChanged();
        say('coming back asks straight away', polls === askedBefore + 1);
        say('and stops the session call', deluge.ui.heartbeat === undefined);

        deluge.ui.update = realUpdate;
        if (deluge.ui.running) clearTimeout(deluge.ui.running);
        if (deluge.ui.heartbeat) clearTimeout(deluge.ui.heartbeat);

        // --- the windows this fork added build ---------------------------
        [
            ['the label settings window', function () { return new Deluge.LabelSettingsWindow(); }],
            ['the tracker settings window', function () { return new Deluge.TrackerSettingsWindow(); }],
            ['the preferences window', function () { return new Deluge.preferences.PreferencesWindow(); }],
        ].forEach(function (pair) {
            var built = true;
            try {
                var window_ = pair[1]();
                if (window_ && window_.destroy) window_.destroy();
            } catch (error) {
                built = false;
                note('  ' + pair[0] + ' threw: ' + error.message);
            }
            say(pair[0] + ' builds', built);
        });

        // --- a duration is days and hours, and still one number ----------
        var duration = new Deluge.DurationField({ fieldLabel: 'x' });
        duration.setValue(36.5);
        say(
            'a delay of 36.5 hours reads back as a day and a half',
            duration.getValue() === 36.5 &&
                duration.days.getValue() === 1 &&
                duration.hours.getValue() === 12.5
        );
        duration.hours.setValue(50);
        duration.onPartChanged();
        say(
            'fifty hours typed into the hours box carries into the days box',
            duration.days.getValue() === 3 && duration.hours.getValue() === 2
        );

        report();
    });
})();
