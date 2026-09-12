/**
 * Deluge.preferences.OtherPage.js
 *
 * Copyright (c) Damien Churchill 2009-2010 <damoxc@gmail.com>
 *
 * This file is part of Deluge and is licensed under GNU General Public License 3.0, or later, with
 * the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */
Ext.namespace('Deluge.preferences');

/**
 * @class Deluge.preferences.Other
 * @extends Ext.form.FormPanel
 */
Deluge.preferences.Other = Ext.extend(Ext.form.FormPanel, {
    constructor: function (config) {
        config = Ext.apply(
            {
                border: false,
                title: _('Other'),
                header: false,
                layout: 'form',
            },
            config
        );
        Deluge.preferences.Other.superclass.constructor.call(this, config);
    },

    initComponent: function () {
        Deluge.preferences.Other.superclass.initComponent.call(this);

        var optMan = deluge.preferences.getOptionsManager();

        // What was here: a release check and an anonymous-statistics
        // upload. Neither has anything behind it. redeluge has no update
        // service to ask and sends nothing anywhere, so both were a
        // preference that could be set and could not mean anything.
        var fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('GeoIP Database'),
            autoHeight: true,
            labelWidth: 80,
            defaultType: 'textfield',
        });
        optMan.bind(
            'geoip_db_location',
            fieldset.add({
                name: 'geoip_db_location',
                fieldLabel: _('Path:'),
                labelSeparator: '',
                width: 200,
            })
        );
    },
});
