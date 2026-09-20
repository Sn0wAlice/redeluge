/**
 * Deluge.Keys.js
 *
 * Copyright (c) Damien Churchill 2009-2010 <damoxc@gmail.com>
 *
 * This file is part of Deluge and is licensed under GNU General Public License 3.0, or later, with
 * the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */

/**
 * @description The torrent status keys that are commonly used around the UI.
 * @class Deluge.Keys
 * @singleton
 */
Deluge.Keys = {
    /**
     * Keys that are used within the torrent grid.
     * <pre>['queue', 'name', 'total_wanted', 'state', 'progress', 'num_seeds',
     * 'total_seeds', 'num_peers', 'total_peers', 'download_payload_rate',
     * 'upload_payload_rate', 'eta', 'ratio', 'distributed_copies',
     * 'is_auto_managed', 'time_added', 'tracker_host', 'download_location', 'last_seen_complete',
     * 'total_done', 'total_uploaded', 'max_download_speed', 'max_upload_speed',
     * 'seeds_peers_ratio', 'total_remaining', 'completed_time', 'time_since_transfer']</pre>
     */
    Grid: [
        'queue',
        'name',
        'total_wanted',
        'state',
        'progress',
        'num_seeds',
        'total_seeds',
        'num_peers',
        'total_peers',
        'download_payload_rate',
        'upload_payload_rate',
        'eta',
        'ratio',
        'distributed_copies',
        'is_auto_managed',
        'time_added',
        'tracker_host',
        'download_location',
        'last_seen_complete',
        'total_done',
        'total_uploaded',
        'max_download_speed',
        'max_upload_speed',
        'seeds_peers_ratio',
        'total_remaining',
        'completed_time',
        'time_since_transfer',
        'label',
        'owner',
        'idle_since',
        'idle_pause_at',
        'idle_resume_at',
        'space_paused',
        'tracker_move_at',
        'tracker_remove_at',
    ],

    /**
     * Keys used in the status tab of the statistics panel.
     * These get updated to include the keys in {@link #Grid}.
     * <pre>['total_done', 'total_payload_download', 'total_uploaded',
     * 'total_payload_upload', 'next_announce', 'tracker_status', 'num_pieces',
     * 'piece_length', 'is_auto_managed', 'active_time', 'seeding_time', 'time_since_transfer',
     * 'seed_rank', 'last_seen_complete', 'completed_time', 'owner', 'public', 'shared']</pre>
     */
    Status: [
        'total_done',
        'total_payload_download',
        'total_uploaded',
        'total_payload_upload',
        'next_announce',
        'tracker_status',
        'num_pieces',
        'piece_length',
        'is_auto_managed',
        'active_time',
        'seeding_time',
        'time_since_transfer',
        'seed_rank',
        'last_seen_complete',
        'completed_time',
        'owner',
        'public',
        'shared',
    ],

    /**
     * Keys used in the files tab of the statistics panel.
     * <pre>['files', 'file_progress', 'file_priorities']</pre>
     */
    Files: ['files', 'file_progress', 'file_priorities'],

    /**
     * Keys used in the peers tab of the statistics panel.
     * <pre>['peers']</pre>
     */
    Peers: ['peers'],

    /**
     * Keys used in the details tab of the statistics panel.
     */
    Details: [
        'name',
        'download_location',
        'total_size',
        'num_files',
        'message',
        'tracker_host',
        'comment',
        'creator',
    ],

    /**
     * Keys used in the options tab of the statistics panel.
     * <pre>['max_download_speed', 'max_upload_speed', 'max_connections', 'max_upload_slots',
     *  'is_auto_managed', 'stop_at_ratio', 'stop_ratio', 'remove_at_ratio', 'private',
     *  'prioritize_first_last']</pre>
     */
    Options: [
        'max_download_speed',
        'max_upload_speed',
        'max_connections',
        'max_upload_slots',
        'is_auto_managed',
        'stop_at_ratio',
        'stop_ratio',
        'remove_at_ratio',
        'private',
        'prioritize_first_last',
        'move_completed',
        'move_completed_path',
        'super_seeding',
        'label',
        'idle_since',
        'idle_pause_at',
        'idle_resume_at',
        'space_paused',
    ],
};

/**
 * The keys a column needs beyond the one it is drawn from.
 *
 * A renderer usually reads its own field and nothing else; these are the ones
 * that read a second. Kept beside the list rather than inferred, because the
 * cost of being wrong is a blank cell nobody notices until they need it.
 */
Deluge.Keys.AlsoNeeded = {
    num_seeds: ['total_seeds'],
    num_peers: ['total_peers'],
    idle_pause_at: ['idle_resume_at'],
    tracker_remove_at: ['tracker_move_at'],
};

/**
 * The keys the interface needs whatever it is showing.
 *
 * `state` decides the icon, the progress text and which menu items are
 * enabled; `label` decides which rows the hidden-label rule leaves out, and it
 * does that whether or not the Label column is on; `queue` is the order the
 * list falls back to; the last two are what the status bar counts, and it
 * counts them from the grid's own records rather than asking again.
 */
Deluge.Keys.Always = [
    'state',
    'label',
    'queue',
    'idle_resume_at',
    'space_paused',
];

/**
 * The keys to ask for, given the columns the grid is actually showing.
 *
 * Most of a status is columns somebody turned off years ago: of the
 * thirty-five keys the grid used to ask for on every poll, a default layout
 * draws about twenty, and a narrowed one far fewer. What is not drawn is not
 * asked for.
 *
 * The column being sorted on is included even when it is hidden, because the
 * store still sorts by it and a field that stops arriving would quietly
 * scramble the order.
 *
 * @param {Deluge.TorrentGrid} grid
 * @return {Array} the keys, or every key the grid knows when it cannot tell
 */
Deluge.Keys.forGrid = function (grid) {
    var model = grid && grid.getColumnModel && grid.getColumnModel();
    if (!model || !model.config) return Deluge.Keys.Grid;

    var wanted = {};
    var want = function (key) {
        if (key && Deluge.Keys.Grid.indexOf(key) !== -1) wanted[key] = true;
    };

    Ext.each(Deluge.Keys.Always, want);

    Ext.each(model.config, function (column, index) {
        if (model.isHidden(index)) return;
        want(column.dataIndex);
        Ext.each(Deluge.Keys.AlsoNeeded[column.dataIndex] || [], want);
    });

    var store = grid.getStore && grid.getStore();
    var sorted = store && store.getSortState && store.getSortState();
    if (sorted) want(sorted.field);

    var keys = [];
    for (var key in wanted) keys.push(key);
    // Sorted, so that showing and hiding the same column twice asks the same
    // question twice: the server answers a changed question with the whole
    // list, and a list of keys in a different order is a changed question.
    keys.sort();
    return keys;
};

// Merge the grid and status keys together as the status keys contain all the
// grid ones.
Ext.each(Deluge.Keys.Grid, function (key) {
    Deluge.Keys.Status.push(key);
});
